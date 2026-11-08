//! Stage-10 desktop flagship, end to end with **two live PhotoCraft apps**:
//! the Carry-On side captures app A's live session over its loopback control
//! channel, seals a split cut (PhotoCraft's own tiles), streams it into a
//! second core in **budget windows** (the UI's per-tile progress), and restores
//! the session into app B — which is inspected afterwards (document, tool,
//! viewport, selected layers) and screenshotted.
//!
//! Honest scope: **OBSERVED (LOCAL)** — two PhotoCraft processes and two
//! Carry-On cores on ONE machine, loopback only. A second Mac is not used;
//! nothing here is PHYSICAL or a §30 platform claim.
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --example photocraft_ux_live -- <config.json>
//! ```
//! Config:
//! ```json
//! {
//!   "source": {"addr":"127.0.0.1:7898","token_file":"…","read_root":"…","write_root":"…"},
//!   "dest":   {"addr":"127.0.0.1:7899","token_file":"…","read_root":"…","write_root":"…"},
//!   "evidence_dir":"docs/evidence/…",
//!   "window_bytes": 262144,
//!   "encoding": "split"
//! }
//! ```

use carryon_adapter_photocraft::control::ControlChannelDriver;
use carryon_adapter_photocraft::live::reconstruct_carried;
use carryon_adapter_photocraft::model::Workspace;
use carryon_adapter_photocraft::{ControlChannelConfig, PhotocraftAdapter};
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::model::{ActionRequest, AuthorityMode, Budget, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use serde::Deserialize;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const MARKER: &str = "org.carryon.photocraft.ux.v1";
const DONE: &str = "org.carryon.photocraft.ux.done=1";
const MAX_CONNECTIONS: usize = 4096;

#[derive(Debug, Deserialize)]
struct SideCfg {
    addr: String,
    token_file: String,
    read_root: String,
    write_root: String,
}

#[derive(Debug, Deserialize)]
struct Cfg {
    source: SideCfg,
    dest: SideCfg,
    evidence_dir: String,
    #[serde(default = "default_window")]
    window_bytes: u64,
    #[serde(default = "default_encoding")]
    encoding: String,
}

fn default_window() -> u64 {
    256 * 1024
}
fn default_encoding() -> String {
    "split".into()
}

fn connect(side: &SideCfg) -> Result<ControlChannelDriver, String> {
    let token = std::fs::read_to_string(&side.token_file)
        .map_err(|e| format!("read token file: {e}"))?
        .trim()
        .to_string();
    ControlChannelDriver::connect(ControlChannelConfig {
        addr: side.addr.clone(),
        token: carryon_adapter_photocraft::Secret::new(token),
        base_document: PathBuf::new(),
        write_root: PathBuf::from(&side.write_root),
        read_root: PathBuf::from(&side.read_root),
    })
    .map_err(|e| e.to_string())
}

fn feature<'a>(features: &'a [String], key: &str) -> Option<&'a str> {
    features
        .iter()
        .find_map(|f| f.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: photocraft_ux_live <config.json>");
    let cfg: Cfg = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let evidence_dir = PathBuf::from(&cfg.evidence_dir);
    std::fs::create_dir_all(&evidence_dir).unwrap();
    let split = cfg.encoding == "split";

    // ---- 0. Probe the live source session (the UI's card) ---------------------
    let mut src_driver = connect(&cfg.source).expect("connect source app");
    let before = src_driver.live_summary().expect("source live summary");
    println!("source live: {:?}", before);

    // ---- 1. Capture the live session into the adapter --------------------------
    let t_capture = Instant::now();
    let capture = src_driver.capture(split).expect("capture live session");
    let capture_secs = t_capture.elapsed().as_secs_f64();
    println!(
        "captured '{}' (dirty={}): document {} B, unsaved {} B, parts {}, encoding {} in {capture_secs:.3}s",
        capture.report.name,
        capture.report.dirty,
        capture.report.document_bytes,
        capture.report.unsaved_bytes,
        capture.report.parts,
        capture.report.encoding
    );

    // ---- 2. Seal the split cut on the source core -----------------------------
    let src_data = tempfile::tempdir().unwrap();
    let mut source = Core::open(src_data.path()).unwrap();
    let title = format!("PhotoCraft: {}", capture.report.name);
    let adapter = match capture.bundle.clone() {
        Some(bundle) => PhotocraftAdapter::with_split_state(
            title.clone(),
            bundle,
            capture.unsaved.clone(),
            capture.workspace.clone(),
            capture.meta.clone(),
        ),
        None => PhotocraftAdapter::with_state(
            title.clone(),
            capture.document.clone(),
            capture.unsaved.clone(),
            capture.workspace.clone(),
            capture.meta.clone(),
        ),
    };
    let info = source.register_adapter(Box::new(adapter)).unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title,
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    let session_str = session.to_string();
    let sealed = source.cut_authoritative_objects(cut);
    let objects = sealed.len();
    println!(
        "sealed cut #{} with {objects} authoritative objects: {sealed:?}",
        cut.number
    );
    assert!(objects >= 3);

    // ---- 3. The descriptor (data only, like the handoff Hello features) -------
    let workspace_json = serde_json::to_string(&capture.workspace).unwrap();
    let descriptor = vec![
        MARKER.to_string(),
        format!("session={session_str}"),
        format!("cut={}", cut.number),
        format!("objects={objects}"),
        format!("name={}", capture.report.name),
        format!("dirty={}", capture.report.dirty),
        format!("workspace={workspace_json}"),
    ];

    let src_id = DeviceIdentity::generate("photo-ux-source").unwrap();
    let dst_id = DeviceIdentity::generate("photo-ux-dest").unwrap();
    let (st, dt) = pair_devices(&src_id, &dst_id, "2026-11-08T00:00:00Z").unwrap();
    let (st, dt) = (Arc::new(st), Arc::new(dt));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    // ---- 4. Source: serve every budget window, then the done ack --------------
    let feat = descriptor.clone();
    let serve = std::thread::spawn(move || {
        let mut source = source;
        let mut connections = 0usize;
        let mut sent_total = 0u64;
        while connections < MAX_CONNECTIONS {
            connections += 1;
            let Ok(mut net) = Session::accept(&listener, &src_id, st.clone()) else {
                break;
            };
            let Ok(theirs) = net.server_negotiate(feat.clone()) else {
                break;
            };
            if theirs.iter().any(|f| f == DONE) {
                sent_total += net.bytes_sent();
                break;
            }
            // A budget window closes the wire with an Abort and a refused window
            // drops the connection; both are normal events for this loop.
            if let Err(e) = source.serve_cut(&mut net) {
                eprintln!("source window closed: {e}");
            }
            sent_total += net.bytes_sent();
        }
        (connections, sent_total)
    });

    // ---- 5. Destination: progressive budget windows ---------------------------
    let dst_data = tempfile::tempdir().unwrap();
    let mut dest = Core::open(dst_data.path()).unwrap();
    dest.set_chunk_size(Some(64 * 1024)).unwrap();
    let mut windows: Vec<serde_json::Value> = Vec::new();
    let mut parsed: Option<(String, u64, String, bool, Workspace)> = None;
    let t_transfer = Instant::now();
    let mut total_recv = 0u64;
    let mirror_cut;
    let mut window_no = 0usize;
    loop {
        window_no += 1;
        assert!(window_no <= MAX_CONNECTIONS, "too many budget windows");
        let t = Instant::now();
        let mut net = Session::connect(&addr, &dst_id, dt.clone()).expect("dest connect");
        let theirs = net
            .client_negotiate(vec![MARKER.to_string()])
            .expect("negotiate");
        if parsed.is_none() {
            let session = feature(&theirs, "session")
                .expect("descriptor session")
                .to_string();
            let cut: u64 = feature(&theirs, "cut")
                .expect("descriptor cut")
                .parse()
                .unwrap();
            let name = feature(&theirs, "name").unwrap_or("carried").to_string();
            let dirty = feature(&theirs, "dirty") == Some("true");
            let workspace: Workspace = feature(&theirs, "workspace")
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            parsed = Some((session, cut, name, dirty, workspace));
        }
        // §6.8 window. An object larger than the window is refused fail-closed
        // (that is the engine's rule, not a bug): widen until it fits, and show
        // the widening in the UI/evidence.
        let mut window = cfg.window_bytes;
        let outcome = loop {
            dest.set_budget(Budget {
                total_net_bytes: window,
                ..Budget::local_default()
            });
            match dest.import_cut(
                &mut net,
                &parsed.as_ref().unwrap().0,
                parsed.as_ref().unwrap().1,
            ) {
                Ok(o) => break o,
                Err(e) if e.family() == "BUDGET" => {
                    let next = (window * 2).min(512 * 1024 * 1024);
                    println!("window {window} B refused ({e}); widening to {next} B");
                    window = next;
                    // The refused attempt consumed the wire; reconnect.
                    drop(net);
                    net = Session::connect(&addr, &dst_id, dt.clone()).expect("dest reconnect");
                    let _ = net
                        .client_negotiate(vec![MARKER.to_string()])
                        .expect("re-negotiate");
                }
                Err(e) => panic!("import window: {e}"),
            }
        };
        let recv = net.bytes_recv();
        total_recv += recv;
        let secs = t.elapsed().as_secs_f64();
        match outcome {
            carryon_core::ImportOutcome::Completed(n) => {
                windows.push(serde_json::json!({
                    "window": window_no, "completed": true, "mirror_cut": n,
                    "window_bytes_effective": window,
                    "wire_bytes": recv, "seconds": secs,
                }));
                mirror_cut = n;
                drop(net);
                break;
            }
            carryon_core::ImportOutcome::PartialBudget(p) => {
                if window_no == 1 {
                    println!(
                        "window 1 published: {:?}",
                        p.published.iter().map(|(id, _)| id).collect::<Vec<_>>()
                    );
                }
                windows.push(serde_json::json!({
                    "window_bytes_effective": window,
                    "window": window_no, "completed": false,
                    "published": p.published.len(), "deferred": p.deferred.len(),
                    "deferred_bytes": p.deferred_bytes, "wire_bytes": recv, "seconds": secs,
                }));
                println!(
                    "window {window_no}: +{} objects ({} deferred), {recv} B, {secs:.3}s",
                    p.published.len(),
                    p.deferred.len()
                );
            }
            carryon_core::ImportOutcome::Suspended(_) => {
                panic!("unexpected suspend in the UX flow");
            }
        }
        drop(net);
    }
    let transfer_secs = t_transfer.elapsed().as_secs_f64();
    let (remote_session, _remote_cut, name, _dirty, workspace) = parsed.unwrap();

    // Final ack connection: tells the still-serving source that the carry is done.
    {
        let mut net = Session::connect(&addr, &dst_id, dt.clone()).expect("ack connect");
        let _ = net.client_negotiate(vec![MARKER.to_string(), DONE.to_string()]);
        drop(net);
    }
    let (connections, sent_total) = serve.join().unwrap();

    // ---- 6. Restore into the live destination app -----------------------------
    let mirror = Core::mirror_session_id(&remote_session);
    let rows = dest
        .cut_object_digests(carryon_core::ids::CutId {
            session: mirror,
            number: mirror_cut,
        })
        .expect("mirror cut digests");
    let carried = reconstruct_carried(&rows, |hex| {
        dest.read_object_hex(hex).map_err(|e| e.to_string())
    })
    .expect("reconstruct carried state from the destination store");
    let mut dst_driver = connect(&cfg.dest).expect("connect dest app");
    let t_restore = Instant::now();
    let restored = dst_driver
        .restore_into_live(
            carried.unsaved.as_deref(),
            carried.bundle.as_ref(),
            &workspace,
            &name,
            Some("ux-after-restore.png"),
        )
        .expect("restore into the live app");
    let restore_secs = t_restore.elapsed().as_secs_f64();
    println!("restored into app B: {restored:?}");

    // ---- 7. The engine's oracle over the imported state -----------------------
    let adapter_b = match carried.bundle.clone() {
        Some(bundle) => PhotocraftAdapter::with_split_state(
            name.clone(),
            bundle,
            carried.unsaved.clone().unwrap_or_default(),
            workspace.clone(),
            carried.meta.clone(),
        ),
        None => PhotocraftAdapter::with_state(
            name.clone(),
            carried.document.clone().unwrap_or_default(),
            carried.unsaved.clone().unwrap_or_default(),
            workspace.clone(),
            carried.meta.clone(),
        ),
    }
    .with_driver(Box::new(connect(&cfg.dest).expect("dest driver")));
    let info_b = dest.register_adapter(Box::new(adapter_b)).unwrap();
    let action = dest
        .execute_action_as(
            carryon_core::ids::CutId {
                session: mirror,
                number: mirror_cut,
            },
            ActionRequest {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
            &info_b.adapter_id,
        )
        .expect("session.restore on the destination");
    assert!(
        action.oracle.agreed,
        "the destination oracle must agree with the sealed state: {:?}",
        action.oracle
    );

    // ---- 8. Independent verification of the live app B ------------------------
    let after = dst_driver.live_summary().expect("destination live summary");
    let tool_ok = after.tool == workspace.active_tool;
    let zoom_ok = (after.zoom - workspace.viewport.zoom).abs() < 1e-6;
    let layers_ok = after.selected_layers == workspace.selected_layers.len();
    println!(
        "app B live: name={:?} tool={:?} zoom={} selected={} (match tool={tool_ok} zoom={zoom_ok} layers={layers_ok})",
        after.name, after.tool, after.zoom, after.selected_layers
    );

    // ---- 9. Evidence ----------------------------------------------------------
    let copy = |name: &str| -> Option<String> {
        let from = PathBuf::from(&cfg.dest.write_root).join(name);
        if !from.exists() {
            return None;
        }
        let to = evidence_dir.join(name);
        std::fs::copy(&from, &to).ok()?;
        Some(to.display().to_string())
    };
    let shot = copy("ux-after-restore.png");
    let evidence = serde_json::json!({
        "experiment": "stage-10 PhotoCraft desktop flagship UX (two live apps)",
        "label": "OBSERVED (LOCAL)",
        "scope": "two PhotoCraft processes + two Carry-On cores on ONE machine; loopback only",
        "not_claimed": [
            "PHYSICAL (no second machine / second NIC)",
            "platform supported (§30 unrun)",
            "PhotoCraft MCP channel (control channel chosen)"
        ],
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "channel": "desktop JSON control channel (loopback, bearer token file, method whitelist)",
        "encoding": capture.report.encoding,
        "source_session": remote_session,
        "source_live_before": before,
        "capture_seconds": capture_secs,
        "capture": capture.report,
        "objects_sealed": objects,
        "connections": connections,
        "budget_window_bytes": cfg.window_bytes,
        "windows": windows,
        "transfer_seconds": transfer_secs,
        "wire_bytes_sent": sent_total,
        "wire_bytes_received": total_recv,
        "mirror_cut": mirror_cut,
        "restore_seconds": restore_secs,
        "restored": restored,
        "oracle": {
            "checked": action.oracle.checked,
            "agreed": action.oracle.agreed,
            "detail": action.oracle.detail,
        },
        "live_after": after,
        "position_match": {"tool": tool_ok, "zoom": zoom_ok, "selected_layers": layers_ok},
        "screenshot": shot,
        "screenshot_note": "app B's own window, captured by PhotoCraft's ui.screenshot into its automation write root",
        "progressive_note": "each window is the engine's real §6.8 budget admission; the UI shows published/deferred objects per window",
    });
    let out = evidence_dir.join("run.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {}", out.display());
    assert!(
        tool_ok && zoom_ok && layers_ok,
        "live position must match the carried one"
    );
    println!("PHOTOCRAFT_UX_LIVE_OK");
}
