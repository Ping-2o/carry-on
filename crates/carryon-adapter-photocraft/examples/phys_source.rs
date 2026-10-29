//! Physical cross-device carry — SOURCE side (runs on the mac).
//!
//! Pulls the live dirty session out of a running PhotoCraft app (control
//! channel), seals it in a source core, and serves the cut over LAN TLS to the
//! paired destination (the Galaxy A04's `phys_dest` binary). Prints the
//! composite-pixel hash of the carried dirty state — the value the destination
//! must reproduce from the carried bytes (`photocraft-cli convert` on-device).
//!
//! Modes (the l3_handoff.sh convention):
//! ```sh
//! phys_source pin <identity_dir>                        # print this device's pin
//! phys_source serve <id_dir> <data_dir> <listen_addr> <dest_pin> \
//!             <evidence.json> <control_cfg.json>        # the carry
//! ```
//! `control_cfg.json`: {"addr":"127.0.0.1:7878","token_file":"…","read_root":"…","write_root":"…"}

use carryon_adapter_photocraft::{
    model::{PhotocraftMeta, Workspace, PHOTOCRAFT_PIN},
    ControlChannelConfig, ControlChannelDriver, PhotocraftAdapter, PhotocraftDriver,
};
use carryon_core::carryon_net::{DeviceIdentity, Session, TrustStore};
use carryon_core::model::{AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use serde_json::json;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const SESSION_TITLE: &str = "photocraft-phys-carry";

fn load_identity(dir: &std::path::Path) -> DeviceIdentity {
    let key = dir.join("identity.bin");
    if key.exists() {
        // Identity files are written by the same harness; kept raw for the
        // experiment (the engine's real path is platform secure storage, §8.7).
        let bytes = std::fs::read(&key).unwrap();
        return bincode_load(&bytes);
    }
    std::fs::create_dir_all(dir).unwrap();
    let id = DeviceIdentity::generate("mac-phys-source").unwrap();
    std::fs::write(&key, bincode_save(&id)).unwrap();
    id
}

// Tiny serde shim: DeviceIdentity is not Serialize, but it exposes DER bytes.
// Persist {cert_der, key_der(secret, never logged), name}.
fn bincode_save(id: &DeviceIdentity) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct Wire<'a> {
        name: &'a str,
        cert_der: &'a [u8],
        key_der: &'a [u8],
    }
    serde_json::to_vec(&Wire {
        name: "mac-phys-source",
        cert_der: &id.cert_der,
        key_der: id.export_key_der(),
    })
    .unwrap()
}

fn bincode_load(bytes: &[u8]) -> DeviceIdentity {
    #[derive(serde::Deserialize)]
    struct Wire {
        name: String,
        cert_der: Vec<u8>,
        key_der: Vec<u8>,
    }
    let w: Wire = serde_json::from_slice(bytes).unwrap();
    DeviceIdentity::from_der(w.name, w.cert_der, w.key_der).expect("identity parts")
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
            let id = load_identity(&PathBuf::from(&args[2]));
            println!("{}", id.pin().to_hex());
        }
        "serve" => {
            let (id_dir, data_dir, listen, dest_pin, evidence_path, digests_path, cfg_path) = (
                PathBuf::from(&args[2]),
                PathBuf::from(&args[3]),
                args[4].clone(),
                args[5].clone(),
                PathBuf::from(&args[6]),
                PathBuf::from(&args[7]),
                PathBuf::from(&args[8]),
            );
            serve(
                &id_dir,
                &data_dir,
                &listen,
                &dest_pin,
                &evidence_path,
                &digests_path,
                &cfg_path,
            );
        }
        other => panic!("unknown mode {other:?}"),
    }
}

fn serve(
    id_dir: &std::path::Path,
    data_dir: &std::path::Path,
    listen: &str,
    dest_pin: &str,
    evidence_path: &std::path::Path,
    digests_path: &std::path::Path,
    cfg_path: &std::path::Path,
) {
    let id = load_identity(id_dir);
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cfg_path).unwrap()).unwrap();
    let token = std::fs::read_to_string(cfg["token_file"].as_str().unwrap())
        .unwrap()
        .trim()
        .to_string();
    let ccfg = ControlChannelConfig {
        addr: cfg["addr"].as_str().unwrap().into(),
        token: carryon_adapter_photocraft::Secret::new(token),
        base_document: PathBuf::from(cfg["write_root"].as_str().unwrap()).join("base.pcraft"),
        write_root: cfg["write_root"].as_str().unwrap().into(),
        read_root: cfg["read_root"].as_str().unwrap().into(),
    };

    // Pull the live dirty session from the real app.
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
    let meta: PhotocraftMeta = a.meta().expect("meta");
    let workspace_bytes = a.workspace_bytes().expect("workspace_bytes");
    let workspace: Workspace = serde_json::from_slice(&workspace_bytes).unwrap();
    assert!(meta.dirty, "the live document must be dirty at seal time");
    println!(
        "pulled live state: doc {} B, unsaved {} B, rev {}",
        document_bytes.len(),
        unsaved_bytes.len(),
        meta.revision
    );

    // The composite-pixel oracle basis the destination must reproduce from the
    // carried bytes (photocraft-cli convert carried.pcraft → PNG → sha256).
    let composite_sha = composite_of(&unsaved_bytes);

    // Digests of the sealed objects — written BEFORE serving so the shell can
    // push them to the device; the destination reads the objects back from its
    // OWN store by these digests and re-hashes them (independent check).
    let digests = json!({
        "photocraft.document.v1": carryon_adapter_photocraft::model::hex_sha256(&document_bytes),
        "photocraft.unsaved_edits.v1": carryon_adapter_photocraft::model::hex_sha256(&unsaved_bytes),
        "photocraft.meta.v1": carryon_adapter_photocraft::model::hex_sha256(&serde_json::to_vec(&meta).unwrap()),
    });
    std::fs::write(digests_path, serde_json::to_vec_pretty(&digests).unwrap()).unwrap();

    let _ = std::fs::remove_dir_all(data_dir);
    let mut source = Core::open(data_dir).unwrap();
    let info = source
        .register_adapter(Box::new(PhotocraftAdapter::with_state(
            SESSION_TITLE,
            document_bytes.clone(),
            unsaved_bytes.clone(),
            workspace.clone(),
            meta.clone(),
        )))
        .unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id.clone(),
            title: SESSION_TITLE.into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    let cut_num = cut.number;
    let sess_str = session.to_string();
    let _ = cut_num;

    let listener = TcpListener::bind(listen).expect("bind listen addr");
    println!("waiting for destination (session={sess_str})");
    let t = Instant::now();
    let trust = trust_with(dest_pin);
    let mut net = Session::accept(&listener, &id, trust).expect("accept");
    net.server_negotiate(vec![]).unwrap();
    source.serve_cut(&mut net).unwrap();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "carried to destination: {} B in {:.3}s",
        net.bytes_sent(),
        secs
    );

    let evidence = json!({
        "experiment": "physical-carry source side",
        "photocraft_pin": PHOTOCRAFT_PIN,
        "document_bytes": document_bytes.len(),
        "unsaved_bytes": unsaved_bytes.len(),
        "document_sha256": carryon_adapter_photocraft::model::hex_sha256(&document_bytes),
        "unsaved_sha256": carryon_adapter_photocraft::model::hex_sha256(&unsaved_bytes),
        "composite_png_sha256_of_dirty_state": composite_sha,
        "source_meta": meta,
        "bytes_sent": net.bytes_sent(),
        "transfer_seconds": secs,
    });
    std::fs::write(evidence_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {}", evidence_path.display());
    println!("SESSION_STRING={sess_str}");
    println!("COMPOSITE_SHA={composite_sha}");
}

/// Composite PNG sha256 of a `.pcraft` blob, computed through the same export
/// path `photocraft-cli convert` uses on the destination (feature inprocess).
fn composite_of(pcraft: &[u8]) -> String {
    use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
    let work = std::env::temp_dir().join("phys-source-composite");
    let _ = std::fs::create_dir_all(&work);
    std::fs::write(work.join("state.pcraft"), pcraft).unwrap();
    let mut d = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    d.open_under_root("state.pcraft").unwrap();
    let composite = d.composite_bytes_of_active().unwrap();
    carryon_adapter_photocraft::model::hex_sha256(&composite)
}
