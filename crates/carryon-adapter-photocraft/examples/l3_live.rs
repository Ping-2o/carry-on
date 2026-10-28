//! LIVE L3 experiment (Phase 2a): seal a real PhotoCraft session from a live
//! desktop app, carry it through two Carry-On cores over loopback TLS, restore
//! into a SECOND live app, and verify the restored viewport/selection/tool.
//!
//! LOCAL evidence only (one machine, `127.0.0.1`): this is the loopback
//! analogue of `l3_handoff.sh`, with the real PhotoCraft GUI as the app on both
//! ends. Evidence lands in `docs/artcraft-bridge/evidence/l3-live/`.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p carryon-adapter-photocraft --example l3_live -- <config.json>
//! ```
//!
//! Config shape (tokens are read from their files; never logged — EVD-006):
//!
//! ```json
//! {
//!   "source": {"addr":"127.0.0.1:7878","token_file":"…","write_root":"…","read_root":"…"},
//!   "dest":   {"addr":"127.0.0.1:7879","token_file":"…","write_root":"…","read_root":"…"},
//!   "evidence_dir":"docs/artcraft-bridge/evidence/l3-live"
//! }
//! ```

use carryon_adapter_photocraft::{
    model::{self, PhotocraftMeta, Workspace},
    ControlChannelConfig, ControlChannelDriver, PhotocraftAdapter, PhotocraftDriver,
};
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use serde::Deserialize;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct SideCfg {
    addr: String,
    token_file: String,
    write_root: String,
    read_root: String,
}

#[derive(Debug, Deserialize)]
struct Cfg {
    source: SideCfg,
    dest: SideCfg,
    evidence_dir: String,
}

fn connect(side: &SideCfg) -> Result<ControlChannelDriver, String> {
    let cfg = ControlChannelConfig {
        addr: side.addr.clone(),
        token: carryon_adapter_photocraft::Secret::new(
            std::fs::read_to_string(&side.token_file)
                .map_err(|e| format!("read token file: {e}"))?
                .trim()
                .to_string(),
        ),
        base_document: PathBuf::from(&side.write_root).join("base.pcraft"),
        write_root: PathBuf::from(&side.write_root),
        read_root: PathBuf::from(&side.read_root),
    };
    ControlChannelDriver::connect(cfg).map_err(|e| format!("connect {}: {e}", side.addr))
}

/// Parse a JSON literal (not a file).
fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap()
}

fn main() {
    let cfg_path = std::env::args()
        .nth(1)
        .expect("usage: l3_live <config.json>");
    let cfg: Cfg = serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let evidence_dir = PathBuf::from(&cfg.evidence_dir);
    std::fs::create_dir_all(&evidence_dir).unwrap();

    let mut evidence = serde_json::json!({
        "experiment": "live-l3-photocraft",
        "photocraft_pin": model::PHOTOCRAFT_PIN,
        "scope": "LOCAL (loopback, one machine)",
        "token_policy": "tokens read from 0600 files, never logged (EVD-006)",
    });

    // ---- 1. Prepare a recognizable dirty session in live app A ----------------
    let mut a = connect(&cfg.source).expect("connect source app");
    let t0 = Instant::now();

    a.execute_command(
        "file.new",
        &json(r#"{"width":1024,"height":768,"mode":"rgb","depth":8,"background":"white"}"#),
    )
    .expect("file.new");
    a.screenshot("shot-a-prep.png").expect("screenshot a-prep");
    a.save_document("base.pcraft")
        .expect("save base (document object)");
    // Dirtying edits: a new layer, paint on it, a selection, a non-default
    // view and tool.
    a.execute_command("layer.new.layer", &json(r#"{"name":"Carried Layer"}"#))
        .expect("layer.new.layer");
    a.execute_command("layer.select", &json(r#"{"name":"Carried Layer"}"#))
        .expect("layer.select");
    a.execute_command(
        "paint.stroke",
        &json(r##"{"points":[[512,384,1.0]],"size":120,"color":"#ff3333"}"##),
    )
    .expect("paint.stroke");
    a.execute_command(
        "select.rect",
        &json(r#"{"x":160,"y":120,"width":320,"height":200,"mode":"replace","ellipse":false}"#),
    )
    .expect("select.rect");
    a.set_ui(&json(r#"{"tool":"brush"}"#)).expect("ui.set tool");
    a.set_ui(&json(r#"{"zoom":0.5}"#)).expect("ui.set zoom");
    a.set_ui(&json(r#"{"center":[512,384]}"#))
        .expect("ui.set center");
    a.set_ui(&json(r#"{"rotation":15}"#))
        .expect("ui.set rotation");

    // ---- 2. Pull the live state through the driver ----------------------------
    let document_bytes = a.document_bytes().expect("document_bytes");
    let unsaved_bytes = a.unsaved_bytes().expect("unsaved_bytes");
    let meta: PhotocraftMeta = a.meta().expect("meta");
    let workspace_bytes = a.workspace_bytes().expect("workspace_bytes");
    let workspace: Workspace = serde_json::from_slice(&workspace_bytes).unwrap();
    let digests = serde_json::json!({
        "photocraft.document.v1": model::hex_sha256(&document_bytes),
        "photocraft.unsaved_edits.v1": model::hex_sha256(&unsaved_bytes),
        "photocraft.meta.v1": model::hex_sha256(&serde_json::to_vec(&meta).unwrap()),
        "photocraft.workspace.v1": model::hex_sha256(&workspace_bytes),
    });
    println!(
        "state pulled: doc {} B, unsaved {} B, dirty={} rev={}",
        document_bytes.len(),
        unsaved_bytes.len(),
        meta.dirty,
        meta.revision
    );
    assert!(meta.dirty, "the live document must be dirty at seal time");
    assert!(!document_bytes.is_empty(), "base document bytes");
    assert!(!unsaved_bytes.is_empty(), "unsaved snapshot bytes");
    let prep_secs = t0.elapsed().as_secs_f64();
    evidence["source_app"] = serde_json::json!({
        "prep_seconds": prep_secs,
        "document_bytes": document_bytes.len(),
        "unsaved_bytes": unsaved_bytes.len(),
        "revision": meta.revision,
        "dirty": meta.dirty,
        "digests": digests,
        "workspace": workspace_bytes,
    });

    // ---- 3. Source core: seal the cut -----------------------------------------
    let src_dir = tempfile::tempdir().unwrap();
    let mut source = Core::open(src_dir.path()).unwrap();
    let adapter_a = PhotocraftAdapter::with_state(
        "photocraft-live-l3",
        document_bytes.clone(),
        unsaved_bytes.clone(),
        workspace.clone(),
        meta.clone(),
    );
    let info = source.register_adapter(Box::new(adapter_a)).unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id.clone(),
            title: "photocraft-live-l3".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    let sealed = source.cut_authoritative_objects(cut);
    println!("sealed authoritative objects: {sealed:?}");
    assert!(sealed.contains(&"photocraft.document.v1".to_string()));
    assert!(sealed.contains(&"photocraft.unsaved_edits.v1".to_string()));
    assert!(sealed.contains(&"photocraft.meta.v1".to_string()));
    assert!(!sealed.contains(&"photocraft.workspace.v1".to_string()));
    let session_str = session.to_string();
    let cut_num = cut.number;

    // ---- 4. Loopback TLS transfer ----------------------------------------------
    let src_id = DeviceIdentity::generate("live-src").unwrap();
    let dst_id = DeviceIdentity::generate("live-dst").unwrap();
    let (st, dt) = pair_devices(&src_id, &dst_id, "live").unwrap();
    let (st, dt) = (Arc::new(st), Arc::new(dt));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let src_handle = std::thread::spawn(move || {
        let mut net = Session::accept(&listener, &src_id, st).unwrap();
        net.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut net).unwrap();
        net.bytes_sent()
    });

    let t_transfer = Instant::now();
    let dest_dir = tempfile::tempdir().unwrap();
    let mut dest_core = Core::open(dest_dir.path()).unwrap();
    let mut net = Session::connect(&addr, &dst_id, dt).unwrap();
    net.client_negotiate(vec![]).unwrap();
    let outcome = dest_core
        .import_cut(&mut net, &session_str, cut_num)
        .unwrap();
    outcome
        .completed_cut()
        .expect("destination import completes");
    let transfer_secs = t_transfer.elapsed().as_secs_f64();
    let dest_recv = net.bytes_recv();
    let src_sent = src_handle.join().unwrap();
    println!("carried: {src_sent} B sent / {dest_recv} B received in {transfer_secs:.3}s");
    evidence["transfer"] = serde_json::json!({
        "transport": "loopback TLS 1.3",
        "bytes_sent": src_sent,
        "bytes_received": dest_recv,
        "seconds": transfer_secs,
    });

    // ---- 5. Destination reconstruction from the DEST store ---------------------
    // The cut carries AUTHORITATIVE objects only (crates/carryon-core/src/
    // transfer.rs `cut_manifest`: role='authoritative'; the workspace object is
    // Ephemeral/optional and is NOT transferred by `import_cut` — the optional
    // split is the engine's design, prepare.rs:176). The selection is document
    // state serialized inside `.pcraft`, so it arrives WITH the unsaved bytes.
    // The ephemeral workspace (viewport/tool/target layer) is digest-pinned here
    // and carried alongside in this LOCAL experiment; recorded honestly below.
    let mirror = Core::mirror_session_id(&session_str);
    let read_back = |digest: &str| dest_core.read_object_hex(digest).unwrap();
    let rb_doc = read_back(digests["photocraft.document.v1"].as_str().unwrap());
    let rb_unsaved = read_back(digests["photocraft.unsaved_edits.v1"].as_str().unwrap());
    let rb_meta = read_back(digests["photocraft.meta.v1"].as_str().unwrap());
    assert_eq!(rb_doc, document_bytes, "document bytes survive the store");
    assert_eq!(rb_unsaved, unsaved_bytes, "unsaved bytes survive the store");
    assert_eq!(rb_meta, serde_json::to_vec(&meta).unwrap(), "meta survives");
    let workspace_in_dest_store =
        dest_core.has_object_hex(digests["photocraft.workspace.v1"].as_str().unwrap());
    assert!(
        !workspace_in_dest_store,
        "the ephemeral workspace object must NOT be in the authoritative cut"
    );
    println!("destination reconstructed 3 authoritative objects from its own store by digest");
    println!(
        "ephemeral workspace object: not in the cut (optional split verified), carried alongside"
    );
    let rb_workspace = workspace_bytes.clone();

    // ---- 6. Materialize the carried document for app B and open it -------------
    let dest_read_root = PathBuf::from(&cfg.dest.read_root);
    std::fs::create_dir_all(&dest_read_root).unwrap();
    std::fs::write(dest_read_root.join("carried.pcraft"), &rb_unsaved).unwrap();

    let mut b = connect(&cfg.dest).expect("connect dest app");
    let t_open = Instant::now();
    b.open_document("carried.pcraft")
        .expect("app.open carried doc in the destination app");
    let open_secs = t_open.elapsed().as_secs_f64();
    b.screenshot("shot-b-before-restore.png")
        .expect("screenshot before restore");

    // ---- 7. Register the reconstructed adapter and restore --------------------
    let rb_meta_parsed: PhotocraftMeta = serde_json::from_slice(&rb_meta).unwrap();
    let rb_workspace_parsed: Workspace = serde_json::from_slice(&rb_workspace).unwrap();
    let adapter_b = PhotocraftAdapter::with_state(
        "photocraft-live-l3",
        rb_doc.clone(),
        rb_unsaved.clone(),
        rb_workspace_parsed.clone(),
        rb_meta_parsed.clone(),
    )
    .with_driver(Box::new(
        connect(&cfg.dest).expect("connect dest app (driver)"),
    ));
    let info_b = dest_core.register_adapter(Box::new(adapter_b)).unwrap();

    let t_restore = Instant::now();
    let mirror_cut = carryon_core::ids::CutId {
        session: mirror,
        number: cut_num,
    };
    let result = dest_core
        .execute_action_as(
            mirror_cut,
            ActionRequest {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
            &info_b.adapter_id,
        )
        .expect("session.restore through the destination core");
    let restore_secs = t_restore.elapsed().as_secs_f64();
    b.screenshot("shot-b-after-restore.png")
        .expect("screenshot after restore");
    println!(
        "session.restore: oracle.agreed={} live_restore={:?} in {restore_secs:.3}s",
        result.oracle.agreed, result.output["live_restore"]
    );
    assert!(result.oracle.agreed, "restore oracle must agree");
    assert_eq!(result.output["live_restore"], true, "live restore reported");

    // ---- 8. Independent verification: read app B state and compare -----------
    let inspect_b = b.inspect().expect("ui.inspect dest");
    let tool_b = inspect_b["tool"].as_str().unwrap_or_default().to_string();
    let active_b = inspect_b["session"]["active"].as_u64().unwrap_or(0);
    let view_b = &inspect_b["views"][active_b as usize];
    let zoom_b = view_b["zoom"].as_f64().unwrap_or(0.0);
    let rot_b = view_b["rotation"].as_f64().unwrap_or(0.0);
    let center_b = view_b["center"].as_array().cloned().unwrap_or_default();
    let doc_b = &inspect_b["document"];
    let selected_b: Vec<String> = doc_b["selectedLayers"]
        .as_array()
        .map(|a| a.iter().map(|v| v.to_string()).collect())
        .unwrap_or_default();
    let has_selection_b = doc_b["hasSelection"].as_bool().unwrap_or(false);
    let sel_bounds_b = &doc_b["selectionBounds"];
    let layers_b = doc_b["layers"].as_array().cloned().unwrap_or_default();

    let zoom_src = rb_workspace_parsed.viewport.zoom;
    let rot_src = rb_workspace_parsed.viewport.rotation;
    let tool_src = rb_workspace_parsed.active_tool.clone();
    let names_src: Vec<String> = rb_workspace_parsed
        .selected_layers
        .iter()
        .map(|t| t.name.clone())
        .collect();

    let close = |a: f64, b: f64, eps: f64| (a - b).abs() < eps;
    let zoom_ok = close(zoom_b, zoom_src, 1e-3);
    let rot_ok = close(rot_b, rot_src, 1e-3);
    let tool_ok = tool_b == tool_src;
    let sel_ok = !rb_workspace_parsed.selected_layers.is_empty()
        && doc_b["selectedLayers"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false);
    let selection_ok = has_selection_b;
    let layer_ok = layers_b
        .iter()
        .any(|l| l["name"].as_str() == Some("Carried Layer"));
    let checks = serde_json::json!({
        "zoom": zoom_ok,
        "rotation": rot_ok,
        "tool": tool_ok,
        "selected_layers_restored": sel_ok,
        "selection_present": selection_ok,
        "carried_layer_present": layer_ok,
    });

    println!("verification: {checks}");
    println!(
        "  zoom {} vs {} | rot {} vs {} | tool {:?} vs {:?} | selBounds {sel_bounds_b:?}",
        zoom_b, zoom_src, rot_b, rot_src, tool_b, tool_src
    );
    evidence["verification"] = serde_json::json!({
        "checks": checks,
        "dest": {
            "tool": tool_b,
            "zoom": zoom_b,
            "rotation": rot_b,
            "center": center_b,
            "selectedLayers": selected_b,
            "hasSelection": has_selection_b,
            "selectionBounds": sel_bounds_b,
            "layerCount": layers_b.len(),
        },
        "source_workspace": {
            "tool": tool_src,
            "zoom": zoom_src,
            "rotation": rot_src,
            "center": rb_workspace_parsed.viewport.center,
            "selected_layers": names_src,
        },
    });

    evidence["optional_split"] = serde_json::json!({
        "authoritative_transferred_by_engine": ["photocraft.document.v1", "photocraft.unsaved_edits.v1", "photocraft.meta.v1"],
        "ephemeral_workspace": {
            "in_dest_store": workspace_in_dest_store,
            "note": "ephemeral objects are published but not sealed (prepare.rs:176); \
                     import_cut transfers the authoritative cut only. The workspace \
                     (viewport/tool/target layer) was digest-pinned and carried alongside \
                     by this one-machine harness; selection needed no separate object \
                     because .pcraft serializes it as document state."
        }
    });
    evidence["dest_app"] = serde_json::json!({
        "open_seconds": open_secs,
        "restore_seconds": restore_secs,
        "restore_output": result.output,
        "oracle": { "checked": result.oracle.checked, "agreed": result.oracle.agreed },
    });
    evidence["source_driver_transcript"] = serde_json::Value::Array(a.transcript().to_vec());
    evidence["dest_driver_transcript"] = serde_json::Value::Array(b.transcript().to_vec());

    let all_ok = zoom_ok && rot_ok && tool_ok && sel_ok && selection_ok && layer_ok;
    evidence["all_checks_passed"] = serde_json::json!(all_ok);

    // Copy screenshots into the evidence dir (they were written under the
    // dest/source apps' write roots).
    let copy_shot = |from: &PathBuf, name: &str| {
        let to = evidence_dir.join(name);
        if from.exists() {
            std::fs::copy(from, to).ok();
        }
    };
    copy_shot(
        &PathBuf::from(&cfg.source.write_root).join("shot-a-prep.png"),
        "shot-a-prep.png",
    );
    copy_shot(
        &PathBuf::from(&cfg.dest.write_root).join("shot-b-before-restore.png"),
        "shot-b-before-restore.png",
    );
    copy_shot(
        &PathBuf::from(&cfg.dest.write_root).join("shot-b-after-restore.png"),
        "shot-b-after-restore.png",
    );
    // Keep the artifacts under test with the evidence.
    std::fs::write(evidence_dir.join("base.pcraft"), &document_bytes).ok();
    std::fs::write(evidence_dir.join("unsaved-snapshot.pcraft"), &unsaved_bytes).ok();

    let out = evidence_dir.join("transcript.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {}", out.display());
    if !all_ok {
        eprintln!("FAIL: some verification checks did not pass");
        std::process::exit(2);
    }
    println!("ALL CHECKS PASSED");
}
