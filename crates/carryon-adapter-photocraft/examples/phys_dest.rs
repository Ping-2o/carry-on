//! Physical cross-device carry — DESTINATION side (cross-compiled to
//! aarch64-linux-android; runs on the Galaxy A04 via adb shell).
//!
//! No features required: only carryon-core + carryon-net. Imports the served
//! cut into a core on the device, reconstructs the carried objects from the
//! DEVICE's own store by digest, writes `carried.pcraft` for on-device
//! verification (photocraft-cli convert), and records the evidence bundle.
//!
//! Modes (the l3_handoff.sh convention):
//! ```sh
//! phys_dest pin <identity_dir>
//! phys_dest dest <id_dir> <data_dir> <source_addr> <source_pin> <session> <evidence.json>
//! ```

use carryon_core::carryon_net::{DeviceIdentity, Pin, Session, TrustStore};
use carryon_core::Core;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

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
        return DeviceIdentity::from_der(w.name, w.cert_der, w.key_der).expect("identity parts");
    }
    std::fs::create_dir_all(dir).unwrap();
    let id = DeviceIdentity::generate("galaxy-a04-phys-dest").unwrap();
    #[derive(serde::Serialize)]
    struct Wire<'a> {
        name: &'a str,
        cert_der: &'a [u8],
        key_der: &'a [u8],
    }
    std::fs::write(
        &key,
        serde_json::to_vec(&Wire {
            name: "galaxy-a04-phys-dest",
            cert_der: &id.cert_der,
            key_der: id.export_key_der(),
        })
        .unwrap(),
    )
    .unwrap();
    id
}

fn trust_with(pin_hex: &str) -> Arc<TrustStore> {
    let pin = Pin::from_hex(pin_hex).expect("64-hex pin");
    let mut t = TrustStore::new();
    t.pair("source", pin, "2026-10-28T00:00:00Z");
    Arc::new(t)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "pin" => {
            let id = load_identity(Path::new(&args[2]));
            println!("{}", id.pin().to_hex());
        }
        "dest" => {
            let (id_dir, data_dir, addr, src_pin, session, digests_path, evidence_path) = (
                PathBuf::from(&args[2]),
                PathBuf::from(&args[3]),
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                PathBuf::from(&args[7]),
                PathBuf::from(&args[8]),
            );
            dest(
                &id_dir,
                &data_dir,
                &addr,
                &src_pin,
                &session,
                &digests_path,
                &evidence_path,
            );
        }
        other => panic!("unknown mode {other:?}"),
    }
}

fn dest(
    id_dir: &Path,
    data_dir: &Path,
    addr: &str,
    src_pin: &str,
    session_str: &str,
    digests_path: &Path,
    evidence_path: &Path,
) {
    let id = load_identity(id_dir);
    // The digests the SOURCE sealed into the cut (from its evidence file).
    // The transfer already verified every object against these digests at
    // import (two-phase publish); this step reconstructs from the DEVICE's
    // own store, re-hashing everything as an independent check.
    let digests: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(digests_path).unwrap()).unwrap();
    let t = Instant::now();
    let mut net = Session::connect(addr, &id, trust_with(src_pin)).expect("TLS connect to source");
    net.client_negotiate(vec![]).unwrap();

    let _ = std::fs::remove_dir_all(data_dir);
    std::fs::create_dir_all(data_dir).unwrap();
    let mut core = Core::open(data_dir).unwrap();
    let cut_num = 0u64; // the source serves its first cut
    let mirror_cut = core
        .import_cut(&mut net, session_str, cut_num)
        .unwrap()
        .completed_cut()
        .expect("import completes");
    let secs = t.elapsed().as_secs_f64();

    // Reconstruct the carried objects from the DEVICE's own store by digest.
    let mut objects = serde_json::json!({});
    let mut carried_pcraft: Vec<u8> = Vec::new();
    let mut all_match = true;
    for (object_id, expect_hex) in digests.as_object().unwrap() {
        let expect = expect_hex.as_str().unwrap();
        let bytes = core
            .read_object_hex(expect)
            .expect("object in device store");
        let got = carryon_adapter_photocraft::model::hex_sha256(&bytes);
        if got != expect {
            all_match = false;
        }
        objects[object_id.as_str()] = json!({
            "sha256": got,
            "expected": expect,
            "match": got == expect,
            "bytes": bytes.len(),
        });
        if object_id == "photocraft.unsaved_edits.v1" {
            carried_pcraft = bytes;
        }
    }
    assert!(
        all_match,
        "device-store re-hash must match the sealed digests"
    );
    assert!(!carried_pcraft.is_empty(), "unsaved state carried");
    std::fs::write(data_dir.join("carried.pcraft"), &carried_pcraft).unwrap();

    let evidence = json!({
        "experiment": "physical-carry destination side (Galaxy A04)",
        "photocraft_pin": "1f7fc0f53696e54da79b95015abb37d7c22cdd1a",
        "import_seconds": secs,
        "bytes_received": net.bytes_recv(),
        "mirror_cut": mirror_cut,
        "carried_objects": objects,
        "device_store_rehash_all_match": all_match,
        "carried_pcraft_bytes": carried_pcraft.len(),
    });
    std::fs::write(evidence_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!(
        "imported {} B in {:.3}s; carried.pcraft {} B; all digests re-verified",
        net.bytes_recv(),
        secs,
        carried_pcraft.len()
    );
    println!("evidence: {}", evidence_path.display());
}
