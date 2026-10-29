//! PHYSICAL L4 authority transfer — SOURCE side (runs on the mac).
//!
//! The Phase-3 gap-closer: the §21.2 single-writer transfer between the
//! LOCAL-L4 flow (two live apps, loopback) and the PHYSICAL-L3 carry
//! (mac → Galaxy A04). Here the **device becomes the single writer**: the
//! source is a LIVE PhotoCraft app (control channel), the cut — including the
//! document as a SPLIT bundle (TILE-SPLITTING.md) — is served over real LAN
//! TLS, and authority transfers with it; the source drops to read-only.
//!
//! Honest encoding note: the live app's automation save is export→bytes→file
//! (`apps/photocraft/src/services.rs` `save_through`, pin `1f7fc0f5…`), so a
//! control-channel `app.save` can only write zip files — it cannot write the
//! directory bundle the split carry needs. The harness therefore converts the
//! app's SAVED BASE zip into a directory bundle on the same machine with the
//! same pinned engine (`InProcessDriver::save_active_bundle_under_root`);
//! PhotoCraft itself writes the bundle (its own incremental content-addressed
//! store), and the composite-pixel oracle pins content identity.
//!
//! Modes:
//! ```sh
//! phys_l4_source pin <identity_dir>                       # device pin
//! phys_l4_source serve <id_dir> <data_dir> <listen_addr> <dest_pin> \
//!               <evidence.json> <digests.json> <control_cfg.json>  # the carry + transfer
//! phys_l4_source reopen <data_dir> <reopen_evidence.json>  # read-only proof (after)
//! phys_l4_source probe <control_cfg.json> <shot_path>      # relaunched-app alive proof
//! ```

use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{
    DocumentBundle, DocumentEncoding, PhotocraftMeta, Workspace,
};
use carryon_adapter_photocraft::{
    ControlChannelConfig, ControlChannelDriver, PhotocraftAdapter, PhotocraftDriver,
};
use carryon_core::carryon_net::{DeviceIdentity, Session, TrustStore};
use carryon_core::model::{AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use serde_json::json;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const SESSION_TITLE: &str = "photocraft-phys-l4";

fn load_identity(dir: &Path) -> DeviceIdentity {
    let key = dir.join("identity.bin");
    if key.exists() {
        #[derive(serde::Deserialize)]
        struct Wire {
            name: String,
            cert_der: Vec<u8>,
            key_der: Vec<u8>,
        }
        let w: Wire = serde_json::from_slice(&std::fs::read(&key).unwrap()).unwrap();
        DeviceIdentity::from_der(w.name, w.cert_der, w.key_der).expect("identity parts")
    } else {
        std::fs::create_dir_all(dir).unwrap();
        let id = DeviceIdentity::generate("mac-phys-l4-source").unwrap();
        #[derive(serde::Serialize)]
        struct Wire<'a> {
            name: &'a str,
            cert_der: &'a [u8],
            key_der: &'a [u8],
        }
        // key_der is the private key: never logged, 0600 file (EVD-006).
        std::fs::write(
            &key,
            serde_json::to_vec(&Wire {
                name: "mac-phys-l4-source",
                cert_der: &id.cert_der,
                key_der: id.export_key_der(),
            })
            .unwrap(),
        )
        .unwrap();
        id
    }
}

fn trust_with(pin_hex: &str) -> Arc<TrustStore> {
    let pin = carryon_core::carryon_net::Pin::from_hex(pin_hex).expect("64-hex pin");
    let mut t = TrustStore::new();
    t.pair("destination", pin, "2026-10-28T00:00:00Z");
    Arc::new(t)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "pin" => {
            let id = load_identity(Path::new(&args[2]));
            println!("{}", id.pin().to_hex());
        }
        "serve" => serve(
            &PathBuf::from(&args[2]),
            &PathBuf::from(&args[3]),
            &args[4],
            &args[5],
            PathBuf::from(&args[6]),
            PathBuf::from(&args[7]),
            PathBuf::from(&args[8]),
        ),
        "reopen" => reopen(&PathBuf::from(&args[2]), PathBuf::from(&args[3])),
        "probe" => probe(&args[2], &args[3]),
        other => panic!("unknown mode {other:?}"),
    }
}

fn control_cfg(path: &str) -> ControlChannelConfig {
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let token = std::fs::read_to_string(cfg["token_file"].as_str().unwrap())
        .unwrap()
        .trim()
        .to_string();
    ControlChannelConfig {
        addr: cfg["addr"].as_str().unwrap().into(),
        token: carryon_adapter_photocraft::Secret::new(token),
        base_document: PathBuf::from(cfg["write_root"].as_str().unwrap()).join("base.pcraft"),
        write_root: cfg["write_root"].as_str().unwrap().into(),
        read_root: cfg["read_root"].as_str().unwrap().into(),
    }
}

/// Convert the live app's SAVED BASE zip into a PhotoCraft directory bundle
/// (the split-carry encoding) with the same pinned engine, on this machine.
/// NOTE: the driver's fail-closed work root is a process-wide OnceLock, so ALL
/// in-process work in this harness shares `scratch`.
fn split_bundle_of_base(base_zip: &Path, scratch: &Path) -> DocumentBundle {
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).unwrap();
    let mut d = InProcessDriver::new(InProcessConfig {
        work_root: scratch.to_path_buf(),
    })
    .unwrap();
    let name = base_zip.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::copy(base_zip, scratch.join(&name)).unwrap();
    d.open_under_root(&name).unwrap();
    d.save_active_bundle_under_root("base-bundle").unwrap();
    let dir = scratch.join("base-bundle");
    d.bundle_of_dir(&dir).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn serve(
    id_dir: &Path,
    data_dir: &Path,
    listen: &str,
    dest_pin: &str,
    evidence_path: PathBuf,
    digests_path: PathBuf,
    cfg_path: PathBuf,
) {
    let id = load_identity(id_dir);
    let cfg_path_str = cfg_path.display().to_string();
    let ccfg = control_cfg(&cfg_path_str);
    let write_root = ccfg.write_root.clone();

    // Pull the live dirty session from the real app (the Phase-3 phys-carry
    // recipe: base save, dirty edits, selection, tool, viewport).
    let mut a = ControlChannelDriver::connect(ccfg).expect("connect source app");
    a.execute_command(
        "file.new",
        &json!({"width":1024,"height":768,"mode":"rgb","depth":8,"background":"white"}),
    )
    .expect("file.new");
    a.save_document("base.pcraft").expect("save base");
    a.execute_command("layer.new.layer", &json!({"name":"Carried Layer"}))
        .unwrap();
    a.execute_command("layer.select", &json!({"name":"Carried Layer"}))
        .unwrap();
    a.execute_command(
        "paint.stroke",
        &serde_json::from_str(r##"{"points":[[512,384,1.0]],"size":120,"color":"#ff3333"}"##)
            .unwrap(),
    )
    .unwrap();
    a.execute_command(
        "select.rect",
        &json!({"x":160,"y":120,"width":320,"height":200}),
    )
    .unwrap();
    a.set_ui(&json!({"tool":"brush"})).unwrap();
    a.set_ui(&json!({"zoom":0.5})).unwrap();

    let document_bytes = a.document_bytes().expect("document_bytes");
    let unsaved_bytes = a.unsaved_bytes().expect("unsaved_bytes");
    // The control driver labels the encoding from the on-disk base (a zip →
    // Monolithic); this carry IS split (the converted bundle below), so the
    // label must say so BEFORE any bytes are hashed — the adapter's
    // with_split_state pins the same value, keeping meta/auth_hash aligned.
    let mut meta: PhotocraftMeta = a.meta().expect("meta");
    meta.document_encoding = DocumentEncoding::Split;
    let workspace: Workspace = {
        let bytes = a.workspace_bytes().expect("workspace_bytes");
        serde_json::from_slice(&bytes).unwrap()
    };
    assert!(meta.dirty, "the live document must be dirty at seal time");

    // The split encoding of the saved base (see the module doc: the live app
    // can only write zips; the pinned engine converts, on this machine).
    let scratch = data_dir.join("split-scratch");
    let bundle = split_bundle_of_base(&write_root.join("base.pcraft"), &scratch);
    println!(
        "split bundle: manifest {} B, {} content objects",
        bundle.manifest.len(),
        bundle.parts.len()
    );

    // Composite-pixel oracle basis (the device must reproduce this from the
    // carried bytes BEFORE its own edits) — under the SAME scratch root (the
    // driver's OnceLock root; see split_bundle_of_base).
    let composite_sha = composite_of(&unsaved_bytes, &scratch);

    let meta_bytes = serde_json::to_vec(&meta).unwrap();
    let hex = carryon_adapter_photocraft::model::hex_sha256;
    let mut digests = json!({
        "photocraft.document.manifest.v1": hex(&bundle.manifest),
        "photocraft.unsaved_edits.v1": hex(&unsaved_bytes),
        "photocraft.meta.v1": hex(&meta_bytes),
        // NOTE: the workspace object is ephemeral — published on the source
        // but never sealed into the cut (Phase-2a discovery #2), so it never
        // crosses the wire; it is absent from the digests on purpose.
    });
    for (part_id, path) in bundle.part_entries() {
        digests[part_id.as_str()] = json!(hex(&bundle.parts[&path]));
    }
    std::fs::write(&digests_path, serde_json::to_vec_pretty(&digests).unwrap()).unwrap();

    // Seal the cut in a PERSISTENT source core (must survive `reopen`).
    let _ = std::fs::remove_dir_all(data_dir.join("core"));
    let mut source = Core::open(&data_dir.join("core")).unwrap();
    let info = source
        .register_adapter(Box::new(PhotocraftAdapter::with_split_state(
            SESSION_TITLE,
            bundle,
            unsaved_bytes.clone(),
            workspace,
            meta.clone(),
        )))
        .unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: SESSION_TITLE.into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    assert!(source.may_mutate(session), "source owns before transfer");
    let sess_str = session.to_string();
    println!(
        "sealed cut {}; source owns authority (session {sess_str})",
        cut.number
    );

    let listener = TcpListener::bind(listen).expect("bind listen addr");
    println!("waiting for destination (session={sess_str})");
    let t_transfer = Instant::now();
    let trust = trust_with(dest_pin);
    let mut net = Session::accept(&listener, &id, trust).expect("accept");
    net.server_negotiate(vec![]).unwrap();
    let t0 = Instant::now();
    let sent_before = net.bytes_sent();
    source.serve_cut(&mut net).unwrap();
    let carry_secs = t0.elapsed().as_secs_f64();
    let carried = net.bytes_sent() - sent_before;

    // §21.2 steps 1, 4–5, 8 over the same LAN TLS session: proposal bound to
    // the authoritative-state hash; relinquish after the destination's
    // durable acceptance.
    let receipts = source
        .serve_authority_transfer(&mut net, session, cut.number)
        .expect("authority transfer");
    let transfer_secs = t_transfer.elapsed().as_secs_f64();
    assert!(
        !source.may_mutate(session),
        "source read-only after relinquish"
    );
    drop(net);

    let evidence = json!({
        "experiment": "physical-L4 source side (live app, mac)",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "scope": "PHYSICAL (two machines, two NICs; LAN TLS 1.3, pin-based)",
        "document_encoding": "split (manifest + tiles; base zip converted to a directory bundle by the pinned engine on this machine)",
        "base_zip_bytes": document_bytes.len(),
        "unsaved_bytes": unsaved_bytes.len(),
        "source_meta": meta,
        "carried_wire_bytes": carried,
        "carry_seconds": carry_secs,
        "transfer_seconds_total": transfer_secs,
        "authority_receipts": {
            "proposal_id": receipts.proposal_id,
            "new_epoch": receipts.new_epoch,
        },
        "source_read_only_after": !source.may_mutate(session),
        "composite_png_sha256_of_dirty_state": composite_sha,
        "session": sess_str,
        "cut_number": cut.number,
    });
    std::fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    println!(
        "carried {carried} B in {carry_secs:.3}s; authority epoch -> {}",
        receipts.new_epoch
    );
    println!("evidence: {}", evidence_path.display());
    println!("SESSION_STRING={sess_str}");
    println!("COMPOSITE_SHA={composite_sha}");
    // The core dir stays on disk for the `reopen` read-only proof.
    drop(source);
}

/// The read-only proof: reopen the persisted source core AFTER the transfer —
/// authority was durably relinquished, so this device reports read-only and
/// `guard_mutation` must refuse with AUTH_ReadOnly (§21.2 step 8).
fn reopen(data_dir: &Path, evidence_path: PathBuf) {
    let core = Core::open(&data_dir.join("core")).expect("reopen source core");
    let sid = core
        .list_sessions()
        .into_iter()
        .find(|id| {
            core.get_session(*id)
                .map(|s| s.title == SESSION_TITLE)
                .unwrap_or(false)
        })
        .expect("persisted source session survives reopen");
    let may = core.may_mutate(sid);
    let guard = core.guard_mutation(sid).err();
    let state = core.authority_state(sid);
    let guard_msg = guard
        .as_ref()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "NOT REFUSED".into());
    println!("reopened source core: may_mutate={may}, guard_mutation={guard_msg}");
    let evidence = json!({
        "reopened_source_may_mutate": may,
        "reopened_source_guard_refused": guard.is_some(),
        "guard_mutation_error": guard_msg,
        "authority_state": state.map(|s| json!({
            "epoch": s.epoch.0,
            "owner_device": s.owner_device,
            "mode": format!("{:?}", s.mode),
            "ambiguous": s.ambiguous,
        })),
    });
    std::fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    if may || guard.is_none() {
        eprintln!("FAIL: reopened source must be read-only and refuse mutation");
        std::process::exit(2);
    }
}

/// The relaunched-app alive proof: a fresh PhotoCraft process on the mac runs
/// (the editor UI); its paired core holds no authority, so the app is
/// read-only-paired in effect — the CORE is the one that refuses.
fn probe(cfg_path: &str, shot_path: &str) {
    let cfg_path_owned = cfg_path.to_string();
    let cfg = control_cfg(&cfg_path_owned);
    let mut d = ControlChannelDriver::connect(cfg).expect("connect relaunched app");
    let alive = d.execute_command("session.inspect", &json!({})).is_ok();
    d.screenshot(shot_path).ok();
    println!("restarted source app alive: {alive}");
    let evidence = json!({
        "restarted_source_app_alive": alive,
        "note": "the relaunched app runs normally; authority lives in the cores — the device's core owns epoch 1, the mac's core refuses mutation (AUTH_ReadOnly)",
    });
    std::fs::write(
        Path::new(cfg_path).with_extension("probe-evidence.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    if !alive {
        std::process::exit(2);
    }
}

/// Composite PNG sha256 of a `.pcraft` blob, through the same export path the
/// on-device engine uses (feature inprocess). `root` must be the scratch root
/// the first in-process driver in this process already granted.
fn composite_of(pcraft: &[u8], root: &Path) -> String {
    std::fs::write(root.join("dirty-state.pcraft"), pcraft).unwrap();
    let mut d = InProcessDriver::new(InProcessConfig {
        work_root: root.to_path_buf(),
    })
    .unwrap();
    d.open_under_root("dirty-state.pcraft").unwrap();
    let composite = d.composite_bytes_of_active().unwrap();
    carryon_adapter_photocraft::model::hex_sha256(&composite)
}
