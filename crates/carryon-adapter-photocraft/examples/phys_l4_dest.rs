//! PHYSICAL L4 authority transfer — DESTINATION side (cross-compiled to
//! aarch64-linux-android; runs on the Galaxy A04 via adb shell).
//!
//! The device becomes the single writer. Two invocations, so the
//! source-independence proof is as strong as it gets — the SECOND run is a
//! FRESH PROCESS that reconstructs everything from the device's own store:
//!
//! ```sh
//! phys_l4_dest pin <identity_dir>
//! phys_l4_dest take <id_dir> <data_dir> <source_addr> <source_pin> <session> \
//!           <digests.json> <evidence.json>      # import + on-device oracle + §21.2 accept
//! phys_l4_dest continue <data_dir> <work_dir> <session> <digests.json> \
//!           <evidence.json>                     # fresh process: owns + edits source-off
//! ```
//!
//! The `take` run imports the cut over real LAN TLS (with the SPLIT document
//! encoding: manifest + tiles, TILE-SPLITTING.md), reconstructs the carried
//! state from the device's own store by digest, opens it with the ON-DEVICE
//! PhotoCraft engine, proves the composite-pixel oracle, then durably accepts
//! the §21.2 authority transfer. The mac kills its app + proves its core
//! read-only between the two runs. The `continue` run reopens the persistent
//! device core, re-derives the state from the store, verifies it still owns
//! the epoch, and edits (`document.edit` through the core → engine) with the
//! source gone.

use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{DocumentBundle, PhotocraftMeta, Workspace};
use carryon_adapter_photocraft::PhotocraftAdapter;
use carryon_core::carryon_net::{DeviceIdentity, Pin, Session, TrustStore};
use carryon_core::ids::CutId;
use carryon_core::model::{ActionRequest, SessionState};
use carryon_core::Core;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const ADAPTER_ID: &str = "org.carryon.photocraft";

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
        let id = DeviceIdentity::generate("galaxy-a04-phys-l4").unwrap();
        #[derive(serde::Serialize)]
        struct Wire<'a> {
            name: &'a str,
            cert_der: &'a [u8],
            key_der: &'a [u8],
        }
        std::fs::write(
            &key,
            serde_json::to_vec(&Wire {
                name: "galaxy-a04-phys-l4",
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
    let pin = Pin::from_hex(pin_hex).expect("64-hex pin");
    let mut t = TrustStore::new();
    t.pair("source", pin, "2026-10-28T00:00:00Z");
    Arc::new(t)
}

/// The carried state, reconstructed from THIS device's store by digest.
struct Carried {
    bundle: DocumentBundle,
    unsaved: Vec<u8>,
    meta: PhotocraftMeta,
    workspace: Workspace,
}

fn reconstruct(core: &Core, digests: &Value) -> Carried {
    let hex = |v: &Value| {
        let s = v.as_str().expect("digest hex");
        core.read_object_hex(s).expect("object in device store")
    };
    let expect_sha = |v: &Value| v.as_str().expect("digest hex").to_string();
    let mut bundle = DocumentBundle {
        manifest: hex(&digests["photocraft.document.manifest.v1"]),
        parts: BTreeMap::new(),
    };
    let mut all_match = true;
    for (id, digest) in digests.as_object().unwrap() {
        let bytes = hex(digest);
        let got = carryon_adapter_photocraft::model::hex_sha256(&bytes);
        if got != expect_sha(digest) {
            all_match = false;
        }
        if id.starts_with("photocraft.tile.") || id.starts_with("photocraft.blob.") {
            // id = photocraft.<tile|blob>.<blake3>.v1 → path tiles/<blake3>.zst
            let kind = if id.starts_with("photocraft.tile.") {
                "tiles"
            } else {
                "blobs"
            };
            let stem = id
                .trim_start_matches("photocraft.")
                .trim_start_matches("tile.")
                .trim_start_matches("blob.")
                .trim_end_matches(".v1");
            bundle.parts.insert(format!("{kind}/{stem}.zst"), bytes);
        }
    }
    assert!(
        all_match,
        "device-store re-hash must match the sealed digests"
    );
    let unsaved = hex(&digests["photocraft.unsaved_edits.v1"]);
    let meta: PhotocraftMeta =
        serde_json::from_slice(&hex(&digests["photocraft.meta.v1"])).unwrap();
    // The ephemeral workspace is never sealed into the cut (Phase-2a
    // discovery #2): absent from the wire, default on the device. L4 editing
    // needs the document, not the viewport.
    let workspace = Workspace::default();
    Carried {
        bundle,
        unsaved,
        meta,
        workspace,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "pin" => {
            let id = load_identity(Path::new(&args[2]));
            println!("{}", id.pin().to_hex());
        }
        "take" => take(
            &PathBuf::from(&args[2]),
            &PathBuf::from(&args[3]),
            &args[4],
            &args[5],
            &args[6],
            &PathBuf::from(&args[7]),
            &PathBuf::from(&args[8]),
        ),
        "continue" => cont(
            &PathBuf::from(&args[2]),
            &PathBuf::from(&args[3]),
            &args[4],
            &PathBuf::from(&args[5]),
            &PathBuf::from(&args[6]),
        ),
        other => panic!("unknown mode {other:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn take(
    id_dir: &Path,
    data_dir: &Path,
    addr: &str,
    src_pin: &str,
    session_str: &str,
    digests_path: &Path,
    evidence_path: &Path,
) {
    let id = load_identity(id_dir);
    let digests: Value =
        serde_json::from_str(&std::fs::read_to_string(digests_path).unwrap()).unwrap();
    let t = Instant::now();
    let mut net = Session::connect(addr, &id, trust_with(src_pin)).expect("TLS connect to source");
    net.client_negotiate(vec![]).unwrap();

    let _ = std::fs::remove_dir_all(data_dir);
    std::fs::create_dir_all(data_dir).unwrap();
    let mut core = Core::open(data_dir).unwrap();
    let cut_num = 0u64; // the source serves its first cut
    let out = core
        .import_cut(&mut net, session_str, cut_num)
        .expect("import over LAN TLS");
    let mirror_cut = out
        .completed_cut()
        .expect("import completes (physical budget: unlimited)");
    let import_secs = t.elapsed().as_secs_f64();
    let wire = net.bytes_recv();
    println!("imported cut {cut_num} -> mirror {mirror_cut}: {wire} B in {import_secs:.3}s");

    // Reconstruct the carried state from the DEVICE's own store by digest.
    let carried = reconstruct(&core, &digests);
    let mirror = Core::mirror_session_id(session_str);

    // On-device oracle BEFORE any edits: open the carried dirty state with
    // the on-device engine and render — the composite must hash equal to the
    // source's (printed by phys_l4_source as COMPOSITE_SHA).
    let work = data_dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut d = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    std::fs::write(work.join("carried.pcraft"), &carried.unsaved).unwrap();
    d.open_under_root("carried.pcraft").unwrap();
    let composite = d.composite_bytes_of_active().unwrap();
    let composite_sha = carryon_adapter_photocraft::model::hex_sha256(&composite);

    // Register the adapter holding the reconstructed state + the live engine
    // session, then durably accept the §21.2 transfer over the SAME session.
    let adapter = PhotocraftAdapter::with_split_state(
        session_str,
        carried.bundle.clone(),
        carried.unsaved.clone(),
        carried.workspace.clone(),
        carried.meta.clone(),
    )
    .with_driver(Box::new(d));
    let info = core.register_adapter(Box::new(adapter)).unwrap();
    let set = core
        .request_authority_transfer(&mut net, mirror, ADAPTER_ID)
        .expect("authority transfer accepted");
    drop(net);

    let owns = core.may_mutate(mirror);
    let state = core.authority_state(mirror).expect("epoch after transfer");
    println!(
        "authority: epoch -> {}, device owns: {owns}, owner: {}",
        set.new_epoch, state.owner_device
    );
    assert!(owns, "the device must own after the transfer");
    assert_eq!(set.new_epoch, 1, "epoch 0 -> 1");
    assert!(!state.ambiguous);

    let evidence = json!({
        "experiment": "physical-L4 destination take (Galaxy A04)",
        "photocraft_pin": "1f7fc0f53696e54da79b95015abb37d7c22cdd1a",
        "scope": "PHYSICAL (two machines, two NICs; LAN TLS 1.3, pin-based)",
        "import_seconds": import_secs,
        "import_wire_bytes": wire,
        "mirror_cut": mirror_cut,
        "device_store_rehash_all_match": true,
        "composite_png_sha256_on_device": composite_sha,
        "computed_on_device_by": "photocraft-engine embedded in phys_l4_dest (aarch64-linux-android)",
        "authority": {
            "new_epoch": set.new_epoch,
            "device_owns": owns,
            "owner_device": state.owner_device,
            "ambiguous": state.ambiguous,
        },
        "registered_adapter": info.adapter_id,
    });
    std::fs::write(evidence_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {}", evidence_path.display());
    println!("DEVICE_COMPOSITE_SHA={composite_sha}");
    println!("DEVICE_EPOCH={}", set.new_epoch);
}

/// The source-off continuation: a FRESH process reopens the persistent device
/// core, reconstructs the carried state from the store (nothing from the
/// source, which is dead), verifies the epoch, and edits through the core.
fn cont(
    data_dir: &Path,
    work_dir: &Path,
    session_str: &str,
    digests_path: &Path,
    evidence_path: &Path,
) {
    let digests: Value =
        serde_json::from_str(&std::fs::read_to_string(digests_path).unwrap()).unwrap();
    let mut core = Core::open(data_dir).expect("reopen the persistent device core");
    let mirror = Core::mirror_session_id(session_str);
    let sess = core
        .get_session(mirror)
        .expect("mirror session survives the fresh process");
    assert!(
        matches!(sess.state, SessionState::ActionReady),
        "the mirror session must still be ACTION_READY (got {:?})",
        sess.state
    );

    // Everything from the DEVICE's own store; the source is gone.
    let carried = reconstruct(&core, &digests);
    let work = work_dir.to_path_buf();
    std::fs::create_dir_all(&work).unwrap();
    let mut d = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    std::fs::write(work.join("carried.pcraft"), &carried.unsaved).unwrap();
    d.open_under_root("carried.pcraft").unwrap();

    let adapter = PhotocraftAdapter::with_split_state(
        session_str,
        carried.bundle.clone(),
        carried.unsaved.clone(),
        carried.workspace.clone(),
        carried.meta.clone(),
    )
    .with_driver(Box::new(d));
    let info = core.register_adapter(Box::new(adapter)).unwrap();

    let owns = core.may_mutate(mirror);
    assert!(
        owns,
        "the fresh-process device core must still own the epoch"
    );
    let cut = CutId {
        session: mirror,
        number: 0,
    };
    let t = Instant::now();
    let edit = core
        .execute_action_as(
            cut,
            ActionRequest {
                class: "document.edit".into(),
                params: json!({"steps": [
                    ["paint.stroke", {"points": [[300, 300, 1.0]], "size": 80, "color": "#3366ff"}],
                    ["layer.setProps", {"name": "Carried Layer", "opacity": 0.8}]
                ]}),
            },
            &info.adapter_id,
        )
        .expect("document.edit with the source gone");
    let edit_secs = t.elapsed().as_secs_f64();
    let steps_ran = edit.output["steps_ran"].as_u64().unwrap_or(0);
    let revision = edit.output["revision"].as_u64().unwrap_or(0);
    // The post-edit composite-pixel hash: `apply_steps` derives it (the same
    // cross-session-comparable projection the Phase-2/3 oracles use).
    let composite_sha = edit.output["unsaved_hash"]
        .as_str()
        .unwrap_or("")
        .to_string();
    println!(
        "source-off edit: steps_ran={steps_ran}, revision={revision}, \
         still owns: {}",
        core.may_mutate(mirror)
    );

    let evidence = json!({
        "experiment": "physical-L4 destination continue, source-off (Galaxy A04)",
        "scope": "PHYSICAL; fresh process; source app killed, source core dropped on the mac",
        "fresh_process": true,
        "device_still_owns": core.may_mutate(mirror),
        "edit_steps_ran": steps_ran,
        "edit_revision": revision,
        "edit_seconds": edit_secs,
        "post_edit_unsaved_hash": composite_sha,
        "document_edit_output": edit.output,
    });
    std::fs::write(evidence_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {}", evidence_path.display());
    if steps_ran != 2 || !core.may_mutate(mirror) {
        eprintln!("FAIL: the source-off edit must run both steps and keep ownership");
        std::process::exit(2);
    }
    println!("PHYSICAL L4 SOURCE-OFF CONTINUATION OK");
}
