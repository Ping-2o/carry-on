//! LIVE L4 experiment (Phase 3, task 2): full single-writer authority transfer
//! between two LIVE PhotoCraft apps over the control channel, then the
//! source-off proof with the real app — the `unsaved_continue.sh` flow
//! (mac → device, kill -9 source) now with the flagship app on both ends.
//!
//! LOCAL evidence (one machine, loopback TLS + two app processes).
//!
//! Flow:
//!  1. launch apps A (:7878) and B (:7879) with fresh tokens + automation roots;
//!  2. build a dirty session in A (layer, paint, selection, tool, viewport);
//!  3. pull state, seal the cut in a PERSISTENT source core;
//!  4. loopback TLS: serve cut + **authority transfer** (prepare/accept/commit
//!     bound to the authoritative-state hash, the adapter's §21.2 proposal);
//!  5. verify: receipts match, epoch 0→1, dest owns (may_mutate), source
//!     read-only;
//!  6. restore into live app B (`session.restore` through the destination
//!     core's photocraft adapter with a live driver) + GUI verification;
//!  7. SOURCE-OFF: SIGKILL app A (std Child::kill), drop the source core,
//!     keep editing on the destination (`document.edit` through the dest core
//!     → live app B), verify the edit lands and dest still owns authority;
//!  8. RESTART the source: reopen the persisted source core → may_mutate ==
//!     false and guard_mutation refused (AUTH read-only); relaunch app A's
//!     process (the editor UI; its paired core holds no authority) and
//!     screenshot it;
//!  9. quit both apps politely; evidence to docs/artcraft-bridge/evidence/l4-live/.
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --example l4_live -- <config.json>
//! ```
//! Config: {"photocraft_bin":"…","work_dir":"…","evidence_dir":"…"}

use carryon_adapter_photocraft::{
    model::{PhotocraftMeta, Workspace},
    ControlChannelConfig, ControlChannelDriver, PhotocraftAdapter, PhotocraftDriver,
};
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use serde::Deserialize;
use serde_json::json;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ADAPTER_ID: &str = "org.carryon.photocraft";
const SESSION_TITLE: &str = "photocraft-live-l4";

#[derive(Debug, Deserialize)]
struct Cfg {
    photocraft_bin: String,
    work_dir: String,
    evidence_dir: String,
}

struct App {
    child: std::process::Child,
    root: PathBuf,
    token: PathBuf,
    port: u16,
}

impl App {
    fn launch(bin: &str, work: &Path, name: &str, port: u16) -> App {
        let root = work.join(name);
        let token = work.join(format!("{name}.token"));
        let _ = std::fs::remove_file(&token);
        std::fs::create_dir_all(&root).unwrap();
        let child = std::process::Command::new(bin)
            .arg("--control")
            .arg(port.to_string())
            .arg("--control-token-file")
            .arg(&token)
            .arg("--automation-read-root")
            .arg(&root)
            .arg("--automation-write-root")
            .arg(&root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("launch photocraft");
        // Wait for the token file + control listener.
        for _ in 0..120 {
            if token.exists() {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    return App {
                        child,
                        root,
                        token,
                        port,
                    };
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        panic!("app {name} did not come up on port {port}");
    }

    fn cfg(&self) -> ControlChannelConfig {
        let token = std::fs::read_to_string(&self.token)
            .unwrap()
            .trim()
            .to_string();
        ControlChannelConfig {
            addr: format!("127.0.0.1:{}", self.port),
            token: carryon_adapter_photocraft::Secret::new(token),
            base_document: self.root.join("base.pcraft"),
            write_root: self.root.clone(),
            read_root: self.root.clone(),
        }
    }

    fn connect(&self) -> ControlChannelDriver {
        ControlChannelDriver::connect(self.cfg()).expect("connect control channel")
    }

    fn quit(&mut self) {
        let _ = self.child.kill(); // the app's own app.quit is tried first by the caller
    }
}

fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap()
}

fn main() {
    let cfg_path = std::env::args()
        .nth(1)
        .expect("usage: l4_live <config.json>");
    let cfg: Cfg = serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let work = PathBuf::from(&cfg.work_dir);
    let evidence_dir = PathBuf::from(&cfg.evidence_dir);
    // Clean up any leaked apps from a previous failed run of this harness
    // (matched by OUR work-dir path in their args, so user instances are
    // untouched). A panicked previous run leaks its children; without this,
    // the old app keeps the port with the OLD token and fresh auth fails.
    let _ = std::process::Command::new("pkill")
        .args(["-9", "-f", &work.display().to_string()])
        .status();
    std::thread::sleep(Duration::from_secs(1));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&evidence_dir).unwrap();
    // Persistent core dirs (must survive the "source restart" step).
    let src_core_dir = work.join("src-core");
    let dst_core_dir = work.join("dst-core");

    let mut evidence = serde_json::json!({
        "experiment": "live-l4-authority-transfer + source-off continuation",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "scope": "LOCAL (one machine; loopback TLS between two cores; two live app processes)",
        "token_policy": "tokens read from 0600 files, never logged (EVD-006)",
    });
    let mut checks = serde_json::json!({});

    // ---- 1. Launch both apps -----------------------------------------------
    let mut app_a = App::launch(&cfg.photocraft_bin, &work, "a", 7878);
    let mut app_b = App::launch(&cfg.photocraft_bin, &work, "b", 7879);
    println!(
        "apps launched (a pid {}, b pid {})",
        app_a.child.id(),
        app_b.child.id()
    );

    // ---- 2. Build the dirty session in app A -------------------------------
    let mut a = app_a.connect();
    a.execute_command(
        "file.new",
        &json(r#"{"width":1024,"height":768,"mode":"rgb","depth":8,"background":"white"}"#),
    )
    .expect("file.new");
    a.save_document("base.pcraft").expect("save base");
    a.execute_command("layer.new.layer", &json(r#"{"name":"Carried Layer"}"#))
        .unwrap();
    a.execute_command("layer.select", &json(r#"{"name":"Carried Layer"}"#))
        .unwrap();
    a.execute_command(
        "paint.stroke",
        &json(r##"{"points":[[512,384,1.0]],"size":120,"color":"#ff3333"}"##),
    )
    .unwrap();
    a.execute_command(
        "select.rect",
        &json(r#"{"x":160,"y":120,"width":320,"height":200}"#),
    )
    .unwrap();
    a.set_ui(&json(r#"{"tool":"brush"}"#)).unwrap();
    a.set_ui(&json(r#"{"zoom":0.5}"#)).unwrap();
    a.set_ui(&json(r#"{"center":[512,384]}"#)).unwrap();
    a.set_ui(&json(r#"{"rotation":15}"#)).unwrap();

    // ---- 3. Pull state, seal in the persistent source core -----------------
    let document_bytes = a.document_bytes().expect("document_bytes");
    let unsaved_bytes = a.unsaved_bytes().expect("unsaved_bytes");
    let meta: PhotocraftMeta = a.meta().expect("meta");
    let workspace_bytes = a.workspace_bytes().expect("workspace_bytes");
    let workspace: Workspace = serde_json::from_slice(&workspace_bytes).unwrap();
    assert!(meta.dirty);
    let digests = json!({
        "photocraft.document.v1": carryon_adapter_photocraft::model::hex_sha256(&document_bytes),
        "photocraft.unsaved_edits.v1": carryon_adapter_photocraft::model::hex_sha256(&unsaved_bytes),
        "photocraft.meta.v1": carryon_adapter_photocraft::model::hex_sha256(&serde_json::to_vec(&meta).unwrap()),
        "photocraft.workspace.v1": carryon_adapter_photocraft::model::hex_sha256(&workspace_bytes),
    });
    evidence["source_app"] = json!({
        "document_bytes": document_bytes.len(),
        "unsaved_bytes": unsaved_bytes.len(),
        "revision": meta.revision,
        "digests": digests,
    });

    let mut source = Core::open(&src_core_dir).unwrap();
    let info = source
        .register_adapter(Box::new(PhotocraftAdapter::with_state(
            SESSION_TITLE,
            document_bytes.clone(),
            unsaved_bytes.clone(),
            workspace.clone(),
            meta.clone(),
        )))
        .unwrap();
    let src_session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id.clone(),
            title: SESSION_TITLE.into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(src_session).unwrap();
    let cut_num = cut.number;
    assert!(
        source.may_mutate(src_session),
        "source owns before transfer"
    );
    let sess_str = src_session.to_string();
    println!("sealed cut {cut_num}; source owns authority");

    // ---- 4. Loopback TLS: serve cut + authority transfer --------------------
    let src_id = DeviceIdentity::generate("l4-src").unwrap();
    let dst_id = DeviceIdentity::generate("l4-dst").unwrap();
    let (st, dt) = pair_devices(&src_id, &dst_id, "l4").unwrap();
    let (st, dt) = (Arc::new(st), Arc::new(dt));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let t_transfer = Instant::now();
    let src_handle = std::thread::spawn(move || {
        let mut net = Session::accept(&listener, &src_id, st).unwrap();
        net.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut net).unwrap();
        // §21.2 steps 1, 4–5, 8: proposal bound to the authoritative-state hash,
        // commit relinquishment after the destination's durable acceptance.
        let receipts = source
            .serve_authority_transfer(&mut net, src_session, cut_num)
            .unwrap();
        assert!(
            !source.may_mutate(src_session),
            "source read-only after relinquish"
        );
        (source, receipts)
    });

    let mut dest = Core::open(&dst_core_dir).unwrap();
    let mut net = Session::connect(&addr, &dst_id, dt).unwrap();
    net.client_negotiate(vec![]).unwrap();
    let mirror_cut_num = dest
        .import_cut(&mut net, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .expect("import completes");
    let mirror = Core::mirror_session_id(&sess_str);
    assert!(!dest.may_mutate(mirror), "mirror starts read-only");

    // The destination adapter must be REGISTERED (holding the reconstructed
    // carried state) before `request_authority_transfer`: the adapter's
    // `accept_authority_transfer` re-derives the authoritative-state hash from
    // the state it imported and must agree with the source's proposal.
    let rb_doc = dest
        .read_object_hex(digests["photocraft.document.v1"].as_str().unwrap())
        .unwrap();
    let rb_unsaved = dest
        .read_object_hex(digests["photocraft.unsaved_edits.v1"].as_str().unwrap())
        .unwrap();
    let rb_meta: PhotocraftMeta = serde_json::from_slice(
        &dest
            .read_object_hex(digests["photocraft.meta.v1"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let dest_read_root = app_b.root.clone();
    std::fs::write(dest_read_root.join("carried.pcraft"), &rb_unsaved).unwrap();

    let mut b = app_b.connect();
    b.open_document("carried.pcraft")
        .expect("open carried doc in app B");
    b.screenshot("shot-b-before-restore.png").ok();

    let adapter_b = PhotocraftAdapter::with_state(
        SESSION_TITLE,
        rb_doc,
        rb_unsaved.clone(),
        workspace.clone(),
        rb_meta.clone(),
    )
    .with_driver(Box::new(app_b.connect()));
    let info_b = dest.register_adapter(Box::new(adapter_b)).unwrap();
    println!("destination reconstructed + registered; opening the transfer");

    // §21.2 destination steps 2–3: validate the proposal against the imported
    // state (hash re-derivation must agree) and durably accept.
    let dest_set = dest
        .request_authority_transfer(&mut net, mirror, ADAPTER_ID)
        .expect("authority transfer accepted");
    let (source, src_set) = src_handle.join().unwrap();
    drop(net);
    let transfer_secs = t_transfer.elapsed().as_secs_f64();

    // ---- 5. Verify the transfer --------------------------------------------
    checks["receipts_match"] = json!(src_set == dest_set);
    checks["epoch_advanced"] = json!(dest_set.new_epoch == 1);
    checks["dest_owns"] = json!(dest.may_mutate(mirror));
    checks["source_read_only"] = json!(!source.may_mutate(src_session));
    let dstate = dest.authority_state(mirror).unwrap();
    checks["dest_owner_local"] = json!(dstate.owner_device == "local");
    checks["dest_not_ambiguous"] = json!(!dstate.ambiguous);
    println!(
        "transfer: receipts match, epoch 0->1, dest owns: {}, source read-only: {}",
        dest.may_mutate(mirror),
        !source.may_mutate(src_session)
    );
    evidence["transfer"] = json!({
        "seconds": transfer_secs,
        "receipts": { "source_epoch": src_set.new_epoch, "dest_epoch": dest_set.new_epoch },
        "dest_state": { "owner": dstate.owner_device, "epoch": dstate.epoch.0, "ambiguous": dstate.ambiguous },
    });

    // ---- 6. Restore into live app B -----------------------------------------
    let mirror_cut = carryon_core::ids::CutId {
        session: mirror,
        number: mirror_cut_num,
    };
    let t_restore = Instant::now();
    let restore = dest
        .execute_action_as(
            mirror_cut,
            ActionRequest {
                class: "session.restore".into(),
                params: json!({}),
            },
            &info_b.adapter_id,
        )
        .expect("session.restore");
    let restore_secs = t_restore.elapsed().as_secs_f64();
    b.screenshot("shot-b-after-restore.png").ok();
    checks["restore_oracle_agreed"] = json!(restore.oracle.agreed);
    checks["restore_live"] = json!(restore.output["live_restore"] == json!(true));

    // GUI verification on app B.
    let inspect_b = b.inspect().expect("ui.inspect B");
    let tool_b = inspect_b["tool"].as_str().unwrap_or_default().to_string();
    let active_b = inspect_b["session"]["active"].as_u64().unwrap_or(0);
    let view_b = &inspect_b["views"][active_b as usize];
    let zoom_b = view_b["zoom"].as_f64().unwrap_or(0.0);
    let rot_b = view_b["rotation"].as_f64().unwrap_or(0.0);
    let sel_bounds_b = inspect_b["document"]["selectionBounds"].clone();
    let has_selection_b = inspect_b["document"]["hasSelection"]
        .as_bool()
        .unwrap_or(false);
    let layers_b = inspect_b["document"]["layers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    checks["restore_zoom"] = json!((zoom_b - 0.5).abs() < 1e-3);
    checks["restore_rotation"] = json!((rot_b - 15.0).abs() < 1e-3);
    checks["restore_tool"] = json!(tool_b == "Brush" || tool_b == "brush");
    checks["restore_selection"] = json!(
        has_selection_b
            && sel_bounds_b
                .as_array()
                .map(|a| a.len() == 4)
                .unwrap_or(false)
    );
    checks["restore_layer"] = json!(layers_b
        .iter()
        .any(|l| l["name"].as_str() == Some("Carried Layer")));
    println!("restored in B: zoom {zoom_b} rot {rot_b} tool {tool_b} sel {sel_bounds_b}");
    evidence["restore"] = json!({
        "seconds": restore_secs,
        "checks_zoom_rotation_tool_selection": {
            "zoom": zoom_b, "rotation": rot_b, "tool": tool_b,
            "selectionBounds": sel_bounds_b, "hasSelection": has_selection_b,
        },
    });

    // ---- 7. SOURCE-OFF: SIGKILL app A, drop source core, continue on dest ---
    let a_pid = app_a.child.id();
    drop(source); // source core object gone; its DB persists on disk
    app_a.child.kill().expect("SIGKILL source app");
    let kill_method = "SIGKILL via std::process::Child::kill() (kill -9)";
    let exited = app_a.child.wait().unwrap();
    checks["source_app_killed"] = json!(!exited.success());
    println!("source app A (pid {a_pid}) killed with {kill_method}");

    // Keep editing on the destination: document.edit through the dest core
    // into live app B. The core holds authority (single writer).
    let t_edit = Instant::now();
    let edit = dest
        .execute_action_as(
            mirror_cut,
            ActionRequest {
                class: "document.edit".into(),
                params: json!({"steps": [["paint.stroke", {"points": [[300, 300, 1.0]], "size": 80, "color": "#3366ff"}]]}),
            },
            &info_b.adapter_id,
        )
        .expect("document.edit after source-off");
    let edit_secs = t_edit.elapsed().as_secs_f64();
    checks["dest_edit_after_source_off"] = json!(true);
    checks["dest_owns_after_source_off"] = json!(dest.may_mutate(mirror));
    // The edit landed in the live app B (its session row moved).
    let row_b = b
        .execute_command("session.inspect", &json!({}))
        .expect("session.inspect B");
    let rev_b = row_b["documents"][active_b as usize]["revision"]
        .as_u64()
        .unwrap_or(0);
    let dirty_b = row_b["documents"][active_b as usize]["dirty"]
        .as_bool()
        .unwrap_or(false);
    // The two apps' `revision` counters are session-relative (the Phase-2
    // lesson) and are NOT comparable across apps — the honest check is the
    // edit outcome itself (steps_ran == 1, B's row dirty) agreeing with B's
    // own session row.
    let steps_ran = edit.output["steps_ran"].as_u64().unwrap_or(0);
    checks["edit_landed_in_app_b"] =
        json!(dirty_b && steps_ran == 1 && edit.output["revision"] == json!(rev_b));
    b.screenshot("shot-b-after-source-off-edit.png").ok();
    println!(
        "destination continued editing with the source gone: revision {rev_b}, dirty {dirty_b}"
    );
    evidence["source_off"] = json!({
        "kill_method": kill_method,
        "source_pid": a_pid,
        "edit_seconds": edit_secs,
        "dest_revision_after_edit": rev_b,
        "dest_dirty": dirty_b,
        "document_edit_output": edit.output,
    });

    // ---- 8. RESTART the source ----------------------------------------------
    // 8a. Reopen the persisted source core: authority was durably relinquished.
    let source2 = Core::open(&src_core_dir).unwrap();
    let sid = source2
        .list_sessions()
        .into_iter()
        .find(|id| {
            source2
                .get_session(*id)
                .map(|s| s.title == SESSION_TITLE)
                .unwrap_or(false)
        })
        .expect("persisted source session survives reopen");
    let may = source2.may_mutate(sid);
    let guard = source2.guard_mutation(sid).err();
    checks["restarted_source_may_mutate_false"] = json!(!may);
    checks["restarted_source_guard_refused"] = json!(guard.is_some());
    let guard_msg = guard
        .as_ref()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "NOT REFUSED".into());
    println!("restarted source core: may_mutate={may}, guard_mutation={guard_msg}");
    evidence["source_restart"] = json!({
        "may_mutate": may,
        "guard_mutation_error": guard_msg,
    });

    // 8b. Relaunch the source APP process (the editor UI; its paired core
    //     holds no authority) and confirm it runs read-only-paired.
    let mut app_a2 = App::launch(&cfg.photocraft_bin, &work, "a", 7878);
    let mut a2 = app_a2.connect();
    let alive = a2.execute_command("session.inspect", &json!({})).is_ok();
    checks["restarted_source_app_alive"] = json!(alive);
    a2.screenshot("shot-a-restarted.png").ok();
    println!("restarted source app alive: {alive} (core refuses mutation: {guard_msg})");

    // ---- 9. Clean shutdown + evidence ---------------------------------------
    // Polite quit through the control channel, then kill as backstop.
    for app in [&app_a2, &app_b] {
        if let Ok(mut d) = {
            let token = std::fs::read_to_string(&app.token).unwrap_or_default();
            let addr = format!("127.0.0.1:{}", app.port);
            let c = ControlChannelConfig {
                addr,
                token: carryon_adapter_photocraft::Secret::new(token.trim()),
                base_document: app.root.join("base.pcraft"),
                write_root: app.root.clone(),
                read_root: app.root.clone(),
            };
            ControlChannelDriver::connect(c)
        } {
            let _ = d.quit();
            std::thread::sleep(Duration::from_millis(1500));
        }
    }
    app_a2.quit();
    app_b.quit();
    app_a.quit();

    let copy_shot = |from: &Path, name: &str| {
        if from.exists() {
            std::fs::copy(from, evidence_dir.join(name)).ok();
        }
    };
    copy_shot(
        &app_b.root.join("shot-b-before-restore.png"),
        "shot-b-before-restore.png",
    );
    copy_shot(
        &app_b.root.join("shot-b-after-restore.png"),
        "shot-b-after-restore.png",
    );
    copy_shot(
        &app_b.root.join("shot-b-after-source-off-edit.png"),
        "shot-b-after-source-off-edit.png",
    );
    copy_shot(
        &app_a2.root.join("shot-a-restarted.png"),
        "shot-a-restarted.png",
    );

    evidence["checks"] = checks.clone();
    let all: Vec<bool> = checks
        .as_object()
        .unwrap()
        .values()
        .filter_map(|v| v.as_bool())
        .collect();
    let all_ok = all.iter().all(|b| *b) && all.len() >= 10;
    evidence["all_checks_passed"] = json!(all_ok);
    evidence["transcript_source_driver"] = json!({ "calls": a.transcript().len() });
    let out = evidence_dir.join("transcript.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {}", out.display());
    for (k, v) in checks.as_object().unwrap() {
        println!("  {k}: {v}");
    }
    if !all_ok {
        eprintln!("FAIL: some checks did not pass");
        std::process::exit(2);
    }
    println!("ALL L4 CHECKS PASSED");
}
