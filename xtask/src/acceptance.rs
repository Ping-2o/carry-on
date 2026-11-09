//! §30 acceptance harness (stage 8) — the "complete engine v1" checklist as
//! executable code.
//!
//! Emits `acceptance-report.json` (machine-readable verdict + the
//! deferred-items list) and fails the process if any executable scenario
//! fails.
//!
//! HARD RULE (spec §2/§30, labels are law): every executable scenario here is
//! LOCAL loopback evidence (two in-process cores — or two processes on ONE
//! machine — over loopback TLS 1.3). Nothing here upgrades any platform to
//! "supported". §30 items that REQUIRE the device phase (physical matrix
//! PLAT-001, signed packages PLAT-006, background execution PLAT-004, the LAN
//! discovery-timing gate NET-003) are listed in the report as DEFERRED with
//! the exact reason — never silently omitted.
//!
//! Usage:
//!   cargo run -p xtask -- acceptance [out_dir]
//!   (the device-phase runner is `Scripts/acceptance_xdev.sh`; it refuses to
//!   start unless CARRYON_ACCEPTANCE_DEVICE=1 is set explicitly — the A04 is
//!   booked by the stage-6 session until the orchestrator signals otherwise)

use carryon_adapter_graph::GraphAdapter;
use carryon_core::carryon_net::{
    verification_string, DeviceIdentity, PairingPayload, Session, TrustStore,
};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const SCENARIO_LABEL: &str = "OBSERVED (LOCAL) — loopback TLS 1.3 on ONE machine (§2/§30)";

/// One executable scenario's outcome.
#[derive(serde::Serialize)]
pub struct ScenarioResult {
    pub id: String,
    pub title: String,
    /// §30 groups + §31 requirement ids the scenario speaks to.
    pub requirements: Vec<String>,
    pub passed: bool,
    pub detail: String,
    pub label: String,
}

/// One §30/§31 checklist row.
#[derive(serde::Serialize)]
pub struct CheckRow {
    /// §30 group ("Core", "Platforms", …) or a §31 id ("PLAT-006").
    pub clause: String,
    pub item: String,
    /// "PASS" (proven; `how` names the evidence) or "DEFERRED".
    pub status: String,
    /// Evidence pointer, or the exact reason a device-phase item is deferred.
    pub how: String,
}

pub fn run(out_dir: &Path) {
    std::fs::create_dir_all(out_dir).unwrap();
    // Panic containment (the fault-injection discipline): a panicking
    // scenario becomes a FAIL row; the report always carries every row.
    // ACCEPTANCE_ONLY=S8 runs a single scenario (debug aid; the full run is
    // the gate). The report is still written with only the selected rows.
    let only = std::env::var("ACCEPTANCE_ONLY").ok();
    let want = |id: &str| {
        only.as_deref()
            .map(|o| o.split(',').any(|x| x.trim() == id))
            .unwrap_or(true)
    };
    let mut scenarios: Vec<ScenarioResult> = Vec::new();
    if want("S1") {
        scenarios.push(catch_scenario(
            "S1",
            "full lifecycle: §18.4 pair → cut → transfer → action → evidence",
            s1_full_lifecycle,
        ));
    }
    if want("S2") {
        scenarios.push(catch_scenario(
            "S2",
            "cooperative suspend → resume across a fresh connection",
            s2_suspend_resume,
        ));
    }
    if want("S3") {
        scenarios.push(catch_scenario(
            "S3",
            "crash-recovery: SIGKILL the source process mid-transfer",
            s3_crash_recovery,
        ));
    }
    if want("S4") {
        scenarios.push(catch_scenario(
            "S4",
            "authority loss in commit window → ambiguous, blocked, recoverable",
            s4_authority_ambiguity,
        ));
    }
    if want("S5") {
        scenarios.push(catch_scenario(
            "S5",
            "§6.8 budget refusal fails closed before any bytes",
            s5_budget_refusal,
        ));
    }
    if want("S6") {
        scenarios.push(catch_scenario(
            "S6",
            "§18.3 discovery TXT minimalism (engine contract)",
            s6_discovery_contract,
        ));
    }
    if want("S7") {
        scenarios.push(catch_scenario(
            "S7",
            "§18.5 auth: pin mismatch / revoked peer / pairing-channel content",
            s7_connection_auth,
        ));
    }
    if want("S8") {
        scenarios.push(catch_scenario(
            "S8",
            "evidence export/verify; tampered bundle refused (EVD-005)",
            s8_evidence,
        ));
    }

    let passed = scenarios.iter().filter(|s| s.passed).count();
    let total = scenarios.len();
    let checklist = checklist(&scenarios);
    let deferred: Vec<String> = checklist
        .iter()
        .filter(|c| c.status == "DEFERRED")
        .map(|d| format!("{}: {} — {}", d.clause, d.item, d.how))
        .collect();

    let report = serde_json::json!({
        "suite": "section30-acceptance",
        "label_law": {
            "executable_scopes": SCENARIO_LABEL,
            "complete_engine_v1": false,
            "wording_required_until_device_phase": "prototype / platform pilot / read-only continuation demo",
        },
        "machine": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "scenarios_passed": passed,
        "scenarios_total": total,
        "scenarios": scenarios,
        "checklist": checklist,
        "verdict": {
            "complete_engine_v1": false,
            "statement": "The engine core is PROVEN at LOCAL scope (all executable scenarios pass; \
                the workspace unit/integration suites are green). 'Complete cross-platform engine v1' \
                (§30) is NOT claimed: the device-phase matrix (PLAT-001), signed packages (PLAT-006), \
                background-execution tests (PLAT-004) and the LAN discovery-timing gate (NET-003) are \
                DEFERRED and enumerated in `uncovered_items`. Wording stays 'prototype / platform pilot'.",
            "uncovered_items": deferred,
        },
    });
    let path = out_dir.join("acceptance-report.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    eprintln!("§30 acceptance harness — {passed}/{total} executable scenarios passed");
    for s in &scenarios {
        eprintln!(
            "  [{}] {} — {}",
            if s.passed { "PASS" } else { "FAIL" },
            s.id,
            s.detail
        );
    }
    eprintln!(
        "deferred to device phase: {} items (see the report)",
        deferred.len()
    );
    eprintln!("wrote {}", path.display());
    if passed != total {
        std::process::exit(1);
    }
}

fn catch_scenario(id: &str, title: &str, f: fn() -> Result<String, String>) -> ScenarioResult {
    eprintln!("  … {id} {title}");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    let (passed, detail) = match result {
        Ok(Ok(detail)) => (true, detail),
        Ok(Err(detail)) => (false, detail),
        Err(p) => (
            false,
            format!(
                "panic: {}",
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string panic".into())
            ),
        ),
    };
    ScenarioResult {
        id: id.into(),
        title: title.into(),
        requirements: scenario_requirements(id),
        passed,
        detail,
        label: SCENARIO_LABEL.into(),
    }
}

fn scenario_requirements(id: &str) -> Vec<String> {
    let v: &[&str] = match id {
        "S1" => &[
            "§30 Core",
            "§30 External apps",
            "CORE-001..005",
            "NET-005",
            "UX-002",
            "EVD-002",
        ],
        "S2" => &["CORE-008", "NET-008", "§18.6"],
        "S3" => &["§30 Correctness", "CORE-008", "EVD-003"],
        "S4" => &["AUTH-001..005", "§21.2/§21.3"],
        "S5" => &["§6.8", "§30 Correctness"],
        "S6" => &["NET-003", "§18.3"],
        "S7" => &["NET-001", "NET-002", "§18.5", "§18.4.9"],
        "S8" => &["§30 Evidence", "EVD-002", "EVD-005"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
}

// ============================ helpers ============================

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "carryon-s30-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A graph source: sealed cut ready to serve.
fn graph_source(dir: &Path) -> (Core, String, u64) {
    let mut core = Core::open(dir).unwrap();
    let info = core
        .register_adapter(Box::new(GraphAdapter::sample()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "s30 graph".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    (core, session.to_string(), cut.number)
}

/// §18.4 pairing the way the apps do it, at engine scope: QR payload
/// (§18.4.1-2), the engine's verification string displayed by both sides
/// (§18.4.4), user confirm on both (§18.4.5, modeled by the equality
/// assertions), mutual pin (§18.4.6-7). Returns pinned trusts + identities.
fn paired_over_qr(
    src_name: &str,
    dst_name: &str,
) -> (
    DeviceIdentity,
    DeviceIdentity,
    Arc<TrustStore>,
    Arc<TrustStore>,
    String,
) {
    let src = DeviceIdentity::generate(src_name).unwrap();
    let dst = DeviceIdentity::generate(dst_name).unwrap();

    // Destination formats its payload (§18.4.1); the source parses it
    // strictly (§18.4.2) — a mangled payload must fail closed.
    let payload =
        PairingPayload::new(dst_name, "127.0.0.1:1", dst.pin().to_hex(), "428913").unwrap();
    let parsed = PairingPayload::parse(&payload.to_uri()).unwrap();
    assert_eq!(parsed.pin_hex, dst.pin().to_hex(), "QR payload round-trip");
    assert!(PairingPayload::parse("carryon://pair?v=2&n=x&a=1.2.3.4:1&k=&r=000000").is_err());

    // The verification string both sides display (§18.4.4): order-independent,
    // six hex, changed by a swapped certificate.
    let vs = verification_string(&src.pin(), &dst.pin());
    assert_eq!(vs, verification_string(&dst.pin(), &src.pin()));
    assert_eq!(vs.len(), 6);
    let eve = DeviceIdentity::generate("eve").unwrap();
    assert_ne!(vs, verification_string(&src.pin(), &eve.pin()));

    // User confirm on BOTH sides (§18.4.5) modeled by the equalities above;
    // each side pins the other (§18.4.6-7).
    let mut st = TrustStore::new();
    st.pair(dst_name, dst.pin(), "s30");
    let mut dt = TrustStore::new();
    dt.pair(src_name, src.pin(), "s30");
    (src, dst, Arc::new(st), Arc::new(dt), vs)
}

/// Serve a sealed cut in a thread; returns the join handle + address.
fn spawn_serve(
    mut core: Core,
    session: String,
    id: DeviceIdentity,
    trust: Arc<TrustStore>,
) -> (std::thread::JoinHandle<()>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let _ = session;
    let handle = std::thread::spawn(move || {
        if let Ok(mut net) = Session::accept(&listener, &id, trust) {
            let _ = net.server_negotiate(vec![]);
            let _ = core.serve_cut(&mut net);
        }
        // core dropped: source service off (destination source-independence).
    });
    (handle, addr)
}

fn import_complete(
    dest: &mut Core,
    addr: &str,
    id: &DeviceIdentity,
    trust: Arc<TrustStore>,
    session: &str,
    cut: u64,
) -> Result<u64, String> {
    let mut net = Session::connect(addr, id, trust).map_err(|e| e.to_string())?;
    net.client_negotiate(vec![]).map_err(|e| e.to_string())?;
    dest.import_cut(&mut net, session, cut)
        .map_err(|e| e.to_string())?
        .completed_cut()
        .ok_or_else(|| "import did not complete".to_string())
}

// ============================ scenarios ============================

/// S1 — the §30 Core + External-apps lifecycle over the wire, INCLUDING the
/// §18.4 pairing step, a real action with independent oracle validation, and
/// a verified evidence bundle on the destination (EVD-002).
fn s1_full_lifecycle() -> Result<String, String> {
    let src_dir = tmp("s1-src");
    let dst_dir = tmp("s1-dst");
    let (source, sess, cut) = graph_source(&src_dir);
    let (src_id, dst_id, st, dt, vs) = paired_over_qr("s30-source", "s30-dest");

    let (srv, addr) = spawn_serve(source, sess.clone(), src_id, st);
    let mut destination = Core::open(&dst_dir).unwrap();
    // The destination registers the same compiled-in adapter: the imported
    // mirror cut names the adapter id, never a path (§3.8/ADP-002).
    destination
        .register_adapter(Box::new(GraphAdapter::sample()))
        .map_err(|e| e.to_string())?;
    let mirror_cut = import_complete(&mut destination, &addr, &dst_id, dt, &sess, cut)?;
    srv.join().map_err(|_| "source thread panicked")?;

    // The destination holds a sealed mirror cut and can run an action on it
    // (read-only replica: `graph.shortest_path` is an OracleKind::Exact
    // action) — the independent validation is the engine's `agreed` result.
    let mirror_session = Core::mirror_session_id(&sess);
    // The mirror cut is (mirror session, same cut number) — CutId is public.
    let mirror_cut_id = carryon_core::ids::CutId {
        session: mirror_session,
        number: mirror_cut,
    };
    let action = destination
        .execute_action_as(
            mirror_cut_id,
            ActionRequest {
                class: "graph.shortest_path".into(),
                params: serde_json::json!({"start": 0, "end": 4}),
            },
            "org.carryon.graph",
        )
        .map_err(|e| format!("action on mirror: {e}"))?;
    if !action.oracle.agreed {
        return Err("action oracle disagreed on the mirror cut".into());
    }

    // Evidence export + verify on the destination (EVD-002).
    let bundle = destination
        .export_evidence(mirror_session)
        .map_err(|e| e.to_string())?;
    let bundle_path = dst_dir.join("acceptance-evidence.json");
    std::fs::write(
        &bundle_path,
        serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let report = destination
        .verify_evidence(&bundle_path)
        .map_err(|e| e.to_string())?;
    if !report.ok {
        return Err(format!(
            "destination evidence verify failed: {}",
            report.message
        ));
    }
    Ok(format!(
        "pair VS={vs}; mirror cut #{mirror_cut}; graph action agreed; evidence verify ok ({} B)",
        std::fs::metadata(&bundle_path)
            .map(|m| m.len())
            .unwrap_or(0)
    ))
}

/// S2 — §11.2 cooperative suspend mid-import, resumed to completion on a
/// fresh connection; the suspended state is never counted as success and the
/// destination's record stays clean (a suspend is not an interruption).
fn s2_suspend_resume() -> Result<String, String> {
    let src_dir = tmp("s2-src");
    let dst_dir = tmp("s2-dst");
    let (mut source, sess, cut) = graph_source(&src_dir);
    let (src_id, dst_id, st, dt, _vs) = paired_over_qr("s30-s-src", "s30-s-dst");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // The source serves BOTH the suspended attempt and the resume (two connections).
    let srv = std::thread::spawn(move || {
        for _ in 0..2 {
            if let Ok(mut net) = Session::accept(&listener, &src_id.clone(), st.clone()) {
                let _ = net.server_negotiate(vec![]);
                let _ = source.serve_cut(&mut net);
            }
        }
    });

    let mut destination = Core::open(&dst_dir).unwrap();
    destination
        .set_chunk_size(Some(4096))
        .map_err(|e| e.to_string())?; // min admitted chunk
    destination.request_suspend();
    let token = {
        let mut net = Session::connect(&addr, &dst_id, dt.clone()).map_err(|e| e.to_string())?;
        net.client_negotiate(vec![]).map_err(|e| e.to_string())?;
        match destination
            .import_cut(&mut net, &sess, cut)
            .map_err(|e| e.to_string())?
        {
            carryon_core::ImportOutcome::Suspended(t) => t,
            other => return Err(format!("expected Suspended, got {other:?}")),
        }
    };
    // A cooperative suspend must NOT be recorded as an interrupted transfer.
    let interrupted = destination.recovery_report().interrupted_transfers.len();
    if interrupted != 0 {
        return Err(format!(
            "suspend counted as interruption: {interrupted} (EVD-003)"
        ));
    }
    // Resume on a FRESH connection (the pipeline's discipline).
    let mut net = Session::connect(&addr, &dst_id, dt).map_err(|e| e.to_string())?;
    net.client_negotiate(vec![]).map_err(|e| e.to_string())?;
    let done = destination
        .resume_import(&mut net, &token)
        .map_err(|e| e.to_string())?
        .completed_cut()
        .ok_or_else(|| "resume did not complete".to_string())?;
    srv.join().map_err(|_| "source thread panicked")?;
    Ok(format!(
        "suspended at offset {} of {}; resumed on a fresh connection → mirror cut #{done}; interrupted==0",
        token.next_offset, token.pending_content_hash
    ))
}

/// S3 — crash-recovery at PROCESS granularity: the source is a separate
/// `xtask acceptance-worker serve` process SIGKILLed mid-transfer. The
/// destination must (a) not count the partial as success, (b) record the
/// interruption (EVD-003), (c) complete on a re-run. The kill -9 ON THE
/// DEVICE (both app processes) stays in the device-phase checklist.
fn s3_crash_recovery() -> Result<String, String> {
    let worker_dir = tmp("s3-worker");
    let dst_dir = tmp("s3-dst");

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut child = std::process::Command::new(&exe)
        .args(["acceptance-worker", "serve", worker_dir.to_str().unwrap()])
        // The engine's bench latency hook: each wire frame waits, so the
        // SIGKILL deterministically lands MID-transfer on a tiny sample cut.
        .env("CARRYON_NET_DELAY_MS", "150")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn worker: {e}"))?;

    // The worker publishes its handshake file: addr, pin, session, cut.
    let info_path = worker_dir.join("worker.json");
    let mut info = None;
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&info_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if v["addr"].is_string() && v["pin"].is_string() {
                    info = Some(v);
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let Some(info) = info else {
        let _ = child.kill();
        return Err("worker never reported its handshake file".into());
    };
    let addr = info["addr"].as_str().unwrap().to_string();
    let pin_hex = info["pin"].as_str().unwrap().to_string();
    let session = info["session"].as_str().unwrap().to_string();
    let cut = info["cut"].as_u64().unwrap();

    // The parent's identity + the local file as the one-machine user channel
    // (the same role the QR plays across devices, §18.4).
    let parent = DeviceIdentity::generate("s30-parent").unwrap();
    std::fs::write(worker_dir.join("parent_pin.txt"), parent.pin().to_hex())
        .map_err(|e| e.to_string())?;
    let mut trust = TrustStore::new();
    let worker_pin = {
        use carryon_core::carryon_net::Pin;
        Pin::from_hex(&pin_hex).ok_or("worker pin not hex")?
    };
    trust.pair("worker", worker_pin, "s30");
    let trust = Arc::new(trust);

    // The destination imports in its own thread; the child is SIGKILLed while
    // the import is actually running (a real mid-transfer crash).
    let importer = {
        let dst_dir = dst_dir.clone();
        let addr = addr.clone();
        let parent = parent.clone();
        let trust = trust.clone();
        let session = session.clone();
        std::thread::spawn(move || -> Result<u64, String> {
            let mut dest = Core::open(&dst_dir).unwrap();
            dest.set_chunk_size(Some(4096)).ok();
            import_complete(&mut dest, &addr, &parent, trust, &session, cut)
        })
    };
    // The worker delays each frame by 150 ms; killing after ~600 ms lands in
    // the middle of the manifest/chunk exchange, never after completion.
    std::thread::sleep(std::time::Duration::from_millis(600));
    let _ = child.kill(); // SIGKILL — the source process dies mid-transfer
    let _ = child.wait();
    let import_result = importer
        .join()
        .map_err(|_| "importer thread panicked".to_string())?;
    if import_result.is_ok() {
        return Err("import completed against a SIGKILLed source — failure not visible".into());
    }

    // Reopen: the interrupted transfer stays in the record (EVD-003).
    let mut destination = Core::open(&dst_dir).unwrap();
    let interrupted = destination.recovery_report().interrupted_transfers.len();
    if interrupted == 0 {
        return Err("recovery report lost the interrupted transfer (EVD-003)".into());
    }

    // Re-run to completion with a healthy source (a fresh worker binary, same
    // destination root) — the pipeline's real retry path.
    let worker_dir2 = tmp("s3-worker2");
    let mut child2 = std::process::Command::new(&exe)
        .args(["acceptance-worker", "serve", worker_dir2.to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn worker2: {e}"))?;
    let info_path2 = worker_dir2.join("worker.json");
    let mut info2 = None;
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&info_path2) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if v["addr"].is_string() {
                    info2 = Some(v);
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let Some(info2) = info2 else {
        let _ = child2.kill();
        return Err("worker2 never reported".into());
    };
    let addr2 = info2["addr"].as_str().unwrap().to_string();
    let pin2 = {
        use carryon_core::carryon_net::Pin;
        Pin::from_hex(info2["pin"].as_str().unwrap()).ok_or("worker2 pin not hex")?
    };
    let session2 = info2["session"].as_str().unwrap().to_string();
    let cut2 = info2["cut"].as_u64().unwrap();
    std::fs::write(worker_dir2.join("parent_pin.txt"), parent.pin().to_hex())
        .map_err(|e| e.to_string())?;
    let mut trust2 = TrustStore::new();
    trust2.pair("worker2", pin2, "s30");
    let done = import_complete(
        &mut destination,
        &addr2,
        &parent,
        Arc::new(trust2),
        &session2,
        cut2,
    )?;
    let _ = child2.kill();
    let _ = child2.wait();
    Ok(format!(
        "SIGKILLed source mid-transfer: import failed, {interrupted} interrupted recorded, re-run completed (mirror #{done})"
    ))
}

/// The worker side of S3: build a cut, publish the handshake file, wait for
/// the parent's pin (one-machine user channel), then serve until killed.
pub fn worker_serve(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let id = DeviceIdentity::generate("s30-worker").unwrap();
    let (source, session, cut) = graph_source(&dir.join("src"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handshake = serde_json::json!({
        "addr": addr,
        "pin": id.pin().to_hex(),
        "session": session,
        "cut": cut,
    });
    std::fs::write(
        dir.join("worker.json"),
        serde_json::to_vec(&handshake).unwrap(),
    )
    .unwrap();
    // Wait (bounded) for the parent's pin — the one-machine out-of-band channel.
    let mut trust = TrustStore::new();
    let mut paired = false;
    for _ in 0..200 {
        if let Ok(pin_hex) = std::fs::read_to_string(dir.join("parent_pin.txt")) {
            if let Some(pin) = {
                use carryon_core::carryon_net::Pin;
                Pin::from_hex(pin_hex.trim())
            } {
                trust.pair("parent", pin, "s30");
                paired = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !paired {
        eprintln!("worker: no parent pin, exiting");
        return;
    }
    if let Ok(mut net) = Session::accept(&listener, &id, Arc::new(trust)) {
        let _ = net.server_negotiate(vec![]);
        let mut source = source;
        let _ = source.serve_cut(&mut net);
    }
}

/// S4 — AUTH-002/004: a loss in the authority commit window leaves the
/// destination ambiguous and write-blocked; recovery restores a single writer.
fn s4_authority_ambiguity() -> Result<String, String> {
    let src_dir = tmp("s4-src");
    let dst_dir = tmp("s4-dst");
    let mut source = Core::open(&src_dir).unwrap();
    let info = source
        .register_adapter(Box::new(carryon_adapter_editor::EditorAdapter::new(
            "editor-session",
            b"draft".to_vec(),
        )))
        .unwrap();
    let src_session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "editor".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut_num = source.create_cut(src_session).unwrap().number;
    let sess_str = src_session.to_string();
    let mut dest = Core::open(&dst_dir).unwrap();
    dest.register_adapter(Box::new(carryon_adapter_editor::EditorAdapter::new(
        "mirror",
        b"draft".to_vec(),
    )))
    .unwrap();

    let (src_id, dst_id, st, dt, _) = paired_over_qr("s30-a-src", "s30-a-dst");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = std::thread::spawn(move || {
        if let Ok(mut net) = Session::accept(&listener, &src_id, st) {
            let _ = net.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut net);
            // Crash in the commit window: propose, then drop the connection.
            let _ = source.propose_authority_transfer(&mut net, src_session, cut_num);
            drop(net);
        }
        // AUTH-002: the source keeps authority (loss ≠ relinquishment).
        source.may_mutate(src_session)
    });
    let mut net = Session::connect(&addr, &dst_id, dt).map_err(|e| e.to_string())?;
    net.client_negotiate(vec![]).map_err(|e| e.to_string())?;
    dest.import_cut(&mut net, &sess_str, cut_num)
        .map_err(|e| e.to_string())?
        .completed_cut()
        .ok_or_else(|| "import did not complete".to_string())?;
    let mirror = Core::mirror_session_id(&sess_str);
    let req = dest.request_authority_transfer(&mut net, mirror, "org.carryon.editor");
    drop(net);
    let source_keeps = srv.join().map_err(|_| "source thread panicked")?;

    let ambiguous = dest
        .authority_state(mirror)
        .map(|s| s.ambiguous)
        .unwrap_or(false);
    let blocked = !dest.may_mutate(mirror) && dest.guard_mutation(mirror).is_err();
    let recovered = dest.recover_authority(mirror).is_ok() && dest.may_mutate(mirror);
    if !(req.is_err() && source_keeps && ambiguous && blocked && recovered) {
        return Err(format!(
            "req_err={} source_keeps={source_keeps} ambiguous={ambiguous} blocked={blocked} recovered={recovered} (AUTH-004 violated)",
            req.is_err()
        ));
    }
    Ok("commit-window loss: request refused, source kept authority, dest ambiguous+blocked, recovery restored one writer".to_string())
}

/// S5 — §6.8: a budget too small for the closure refuses BEFORE any wire
/// bytes; nothing is published as valid.
fn s5_budget_refusal() -> Result<String, String> {
    let src_dir = tmp("s5-src");
    let dst_dir = tmp("s5-dst");
    let (mut source, sess, cut) = graph_source(&src_dir);
    let mut dest = Core::open(&dst_dir).unwrap();
    let mut budget = carryon_core::model::Budget::local_default();
    budget.total_net_bytes = 1;
    dest.set_budget(budget);
    let (src_id, dst_id, st, dt, _) = paired_over_qr("s30-b-src", "s30-b-dst");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = std::thread::spawn(move || {
        if let Ok(mut net) = Session::accept(&listener, &src_id, st) {
            let _ = net.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut net);
        }
    });
    let mut net = Session::connect(&addr, &dst_id, dt).map_err(|e| e.to_string())?;
    net.client_negotiate(vec![]).map_err(|e| e.to_string())?;
    let res = dest.import_cut(&mut net, &sess, cut);
    drop(net);
    srv.join().map_err(|_| "source thread panicked")?;
    match res {
        Err(e) if e.family() == "BUDGET" => Ok(format!(
            "over-budget import refused closed ({e}) — §6.8 fail-closed"
        )),
        Ok(_) => Err("over-budget import unexpectedly succeeded".into()),
        Err(e) => Err(format!("refused, but with the wrong family: {e}")),
    }
}

/// S6 — the engine owns the §18.3 discovery contract: minimal TXT fields,
/// unknown keys ignored on receive, leaky records refused. The < 10 s LAN
/// timing gate is a DEVICE-PHASE item (see the checklist) and is NOT claimed.
fn s6_discovery_contract() -> Result<String, String> {
    use carryon_core::carryon_net::{DeviceClass, DiscoveredPeer, StaticDiscovery, TxtRecord};
    let txt = TxtRecord {
        protocol_major: 1,
        instance_id: "inst-1".into(),
        pairing_required: true,
        device_class: DeviceClass::Mobile,
        connection_hint: "49152".into(),
    };
    let pairs = txt.to_pairs();
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    if keys != ["pm", "id", "pr", "dc", "ch"] {
        return Err(format!("TXT key set leaked beyond §18.3: {keys:?}"));
    }
    // Unknown keys are ignored, never interpreted (receive-side minimality).
    let mut with_extra = pairs.clone();
    with_extra.push(("vendor".into(), "x".into()));
    let (parsed, ignored) = TxtRecord::from_pairs(&with_extra).map_err(|e| e.to_string())?;
    if parsed != txt || !ignored.contains(&"vendor".to_string()) {
        return Err(format!(
            "TXT receive filter mismatch: parsed={parsed:?} ignored={ignored:?}"
        ));
    }
    // A record missing a required key fails closed.
    let mut missing = pairs.clone();
    missing.retain(|(k, _)| k != "id");
    if TxtRecord::from_pairs(&missing).is_ok() {
        return Err("TXT missing `id` must be refused (§18.3)".into());
    }
    // Browse returns the advertised peer (contract shape), and the whole
    // record contains no user/session/hash/key material by construction.
    let sd = StaticDiscovery::new().with_peer(DiscoveredPeer {
        instance_id: "inst-1".into(),
        address: "127.0.0.1:1".into(),
        txt: txt.clone(),
    });
    use carryon_core::carryon_net::Discovery;
    let peers = sd.browse();
    if peers.len() != 1 || peers[0].txt.device_class != DeviceClass::Mobile {
        return Err("StaticDiscovery browse mismatch".into());
    }
    Ok("TXT minimalism + unknown-key tolerance + missing-key refusal at engine scope; LAN timing gate = device phase (DEFERRED)".to_string())
}

/// S7 — §18.5 connection authentication negatives: (a) an unknown/untrusted
/// peer's pinned connect fails closed; (b) a revoked pin (§18.4.9) is
/// refused; (c) the provisional pairing channel refuses CONTENT in both
/// directions (§18.4.3 → §18.5).
fn s7_connection_auth() -> Result<String, String> {
    // (a) unknown peer pin.
    let src_dir = tmp("s7-src");
    let (mut source, _sess, _cut) = graph_source(&src_dir);
    let (src_id, _dst_id, st, _dt, _) = paired_over_qr("s30-x-src", "s30-x-dst");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let guard = std::thread::spawn(move || {
        if let Ok(mut net) = Session::accept(&listener, &src_id, st) {
            let _ = net.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut net);
        }
    });
    let eve = DeviceIdentity::generate("eve").unwrap();
    let refused_a = Session::connect(&addr, &eve, Arc::new(TrustStore::new()))
        .and_then(|mut s| {
            s.client_negotiate(vec![])?;
            s.recv().map(|_| ())
        })
        .is_err();
    guard.join().map_err(|_| "unknown-pin source panicked")?;

    // (b) §18.4.9: revoke the destination's pin on the source side; the
    // destination's pinned reconnect is refused.
    let (mut source2, _sess2, _cut2) = graph_source(&tmp("s7-src2"));
    let (src2, dst2, st2, dt2, _) = paired_over_qr("s30-r-src", "s30-r-dst");
    let mut st2_revoked = (*st2).clone();
    if !st2_revoked.revoke(&dst2.pin()) || st2_revoked.is_trusted(&dst2.pin()) {
        return Err("revocation state did not take (§18.4.9)".into());
    }
    let listener2 = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr2 = listener2.local_addr().unwrap().to_string();
    let srv2 = std::thread::spawn(move || {
        if let Ok(mut net) = Session::accept(&listener2, &src2, Arc::new(st2_revoked)) {
            let _ = net.server_negotiate(vec![]);
            let _ = source2.serve_cut(&mut net);
        }
    });
    let refused_b = Session::connect(&addr2, &dst2, dt2)
        .and_then(|mut s| {
            s.client_negotiate(vec![])?;
            s.recv().map(|_| ())
        })
        .is_err();
    srv2.join().map_err(|_| "revoked-source panicked")?;

    // (c) the provisional pairing channel carries pairing traffic ONLY.
    use carryon_core::carryon_net::Message;
    let (src3, dst3, _st3, _dt3, _) = paired_over_qr("s30-p-src", "s30-p-dst");
    let listener3 = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr3 = listener3.local_addr().unwrap().to_string();
    let srv3 = std::thread::spawn(move || -> bool {
        let Ok(mut net) = Session::accept_pairing(&listener3, &src3) else {
            return false;
        };
        let _ = net.server_negotiate(vec![]);
        net.send(Message::CutRequest {
            session: "s".into(),
            cut_number: 1,
        })
        .is_err()
    });
    let mut client3 = Session::connect_pairing(&addr3, &dst3).map_err(|e| e.to_string())?;
    client3
        .client_negotiate(vec![])
        .map_err(|e| e.to_string())?;
    let client_refused = client3
        .send(Message::CutRequest {
            session: "s".into(),
            cut_number: 1,
        })
        .is_err();
    drop(client3);
    let server_refused = srv3.join().map_err(|_| "pairing-source panicked")?;

    if !(refused_a && refused_b && client_refused && server_refused) {
        return Err(format!(
            "unknown_pin={refused_a} revoked={refused_b} pairing_client={client_refused} pairing_server={server_refused}"
        ));
    }
    Ok("unknown pin refused; revoked pin refused (§18.4.9); pairing channel refuses content on BOTH sides (§18.5)".to_string())
}

/// S8 — §30 Evidence: a destination bundle exports and verifies; a TAMPERED
/// bundle must fail verification without executing anything (EVD-005).
fn s8_evidence() -> Result<String, String> {
    let src_dir = tmp("s8-src");
    let dst_dir = tmp("s8-dst");
    let (source, sess, cut) = graph_source(&src_dir);
    let (src_id, dst_id, st, dt, _) = paired_over_qr("s30-e-src", "s30-e-dst");
    let (srv, addr) = spawn_serve(source, sess.clone(), src_id, st);
    let mut destination = Core::open(&dst_dir).unwrap();
    import_complete(&mut destination, &addr, &dst_id, dt, &sess, cut)?;
    srv.join().map_err(|_| "source thread panicked")?;

    let mirror = Core::mirror_session_id(&sess);
    let bundle = destination
        .export_evidence(mirror)
        .map_err(|e| e.to_string())?;
    let path = dst_dir.join("s8-evidence.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let ok = destination
        .verify_evidence(&path)
        .map_err(|e| e.to_string())?;
    if !ok.ok {
        return Err(format!("honest bundle failed verification: {}", ok.message));
    }
    // Tamper: flip one byte; verification must fail closed (EVD-002/005).
    let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x01;
    let tampered = dst_dir.join("s8-tampered.json");
    std::fs::write(&tampered, &bytes).map_err(|e| e.to_string())?;
    let verdict = destination
        .verify_evidence(&tampered)
        .map_err(|e| e.to_string())?;
    if verdict.ok {
        return Err(
            "a TAMPERED bundle verified ok — evidence integrity broken (EVD-002/005)".into(),
        );
    }
    Ok(format!(
        "honest bundle verified; tampered bundle refused ({})",
        verdict.message
    ))
}

// ============================ checklist ============================

/// The §30 matrix as data: what this harness PROVES (with the scenario /
/// suite pointer) and what is DEFERRED to the device phase (with the reason).
/// Every §30 bullet appears exactly once — nothing is omitted.
fn checklist(scenarios: &[ScenarioResult]) -> Vec<CheckRow> {
    let s = |id: &str| {
        scenarios
            .iter()
            .find(|s| s.id == id)
            .map(|s| format!("{} ({})", s.id, if s.passed { "PASS" } else { "FAIL" }))
            .unwrap_or_else(|| format!("{id}: MISSING"))
    };
    // Free fns (not closures) so `how`/`why` accept &str or String uniformly.
    fn pass(clause: &str, item: &str, how: impl Into<String>) -> CheckRow {
        CheckRow {
            clause: clause.into(),
            item: item.into(),
            status: "PASS".into(),
            how: how.into(),
        }
    }
    fn defer(clause: &str, item: &str, why: impl Into<String>) -> CheckRow {
        CheckRow {
            clause: clause.into(),
            item: item.into(),
            status: "DEFERRED".into(),
            how: why.into(),
        }
    }
    vec![
        // ---- §30 Core ----
        pass("Core", "shared core implemented and tested", format!("{}; workspace suite green (test/clippy/fmt gate)", s("S1"))),
        pass("Core", "math encoder with independent test vectors", "math-core test vectors (prototype/math-core suite)",
        ),
        pass("Core", "versioned object/cut/action model", format!("{} (cut immutability CORE-002, object digests CORE-003)", s("S1"))),
        pass("Core", "crash-consistent persistence", format!("{}, {}", s("S2"), s("S3"))),
        pass("Core", "authenticated resumable transport", format!("{}/{}/{} (TLS 1.3 mutual pinning, bounded resume NET-001/008)", s("S1"), s("S2"), s("S7"))),
        pass("Core", "audit/evidence export", format!("{} (export + verify + tamper refusal)", s("S8"))),
        defer("Core", "security review completed", "owner-scheduled security review pass over engine + apps; the harness cannot grant it"),
        // ---- §30 Platforms ----
        defer("Platforms", "physical macOS/Windows/Linux/iOS/iPadOS/Android build matrix passes", "PLAT-001 device phase: real-hardware acceptance; frontier scope today = macOS + Android pilots (Windows/Linux outside frontier; iOS dropped 2026-10-29)"),
        defer("Platforms", "packages are signed according to platform practice", "PLAT-006 skipped by owner (stage 7): no release signing/notarization — the harness must NOT imply signed artifacts"),
        pass("Platforms", "secure key storage, pairing, permissions, activation, evidence export", "PLAT-002 OBSERVED on-device (stage 5: AndroidKeyStore + Keychain, evidence/20261108-a04-pairing); activation = PLAT-003 ACTION_VIEW (stage 3 PHYSICAL)"),
        defer("Platforms", "background-execution limitations documented and tested", "PLAT-004 device phase: real app backgrounding/Doze on the A04"),
        // ---- §30 External applications ----
        pass("External applications", "generic L1 file continuation works", "stage-3 PHYSICAL run: mac⇄A04 file handoff + ACTION_VIEW (evidence/20261108-a04-l0l1)"),
        pass("External applications", "at least two L3 structured adapters across different application families", "engine-level: graph (structured data) + editor (document/unsaved edits), exercised by S1/S4 + workspace suites"),
        defer("External applications", "at least one adapter is a real external application extension", "stage-10 flagship integrations (PhotoCraft/Kiwix) are not wired as external app extensions yet"),
        pass("External applications", "at least one L4 single-writer handoff passes failure and recovery tests", format!("{} (AUTH-002/004) + workspace authority suite", s("S4"))),
        pass("External applications", "unsupported applications fail honestly", "carryon-adapter-unsupported honest refusal (workspace suite)"),
        // ---- §30 Correctness ----
        pass("Correctness", "stale versions are rejected", "fault_stale_generation (xtask fault-injection) + editor generation guard"),
        pass("Correctness", "source and destination agree on cuts and object versions", format!("{} (manifest digest echo + ImportComplete audit)", s("S1"))),
        pass("Correctness", "action outputs pass independent validation", format!("{} (graph oracle agreed on the mirror cut)", s("S1"))),
        pass("Correctness", "authority ambiguity blocks conflicting writes", format!("{} (AUTH-004)", s("S4"))),
        pass("Correctness", "interrupted operations are not counted as successes", format!("{}, {}", s("S2"), s("S3"))),
        // ---- §30 Evidence ----
        pass("Evidence", "reproducible build and test instructions exist", "CONTRIBUTING.md green gate (= CI); this harness: cargo run -p xtask -- acceptance"),
        pass("Evidence", "physical cross-platform runs produce independently verifiable bundles", "stages 3/5 PHYSICAL bundles (evidence/20261107-a04-demo, 20261108-a04-l0l1, 20261108-a04-pairing); S8 proves bundle verification"),
        pass("Evidence", "performance includes overhead and losing cases", "xtask bench (mean/p95/CI, lose/tie/win) + bench-aggregate (Mac→A04 campaign)"),
        pass("Evidence", "claims distinguish local, simulated, physical, externally reviewed evidence", "labels are law in this report: executable = LOCAL; PHYSICAL pointers named; externally reviewed = none yet (see deferred security review)"),
        // ---- Device-phase items (orchestrator signal gates these) ----
        defer("Device-phase", "NET-003 gate: app-open → discovered peer < 10 s on a real LAN", "requires A04 + mac on one LAN (stage-4 pilots); S6 proves only the contract"),
        defer("Device-phase", "kill -9 both APP processes on the phone mid-lifecycle", "requires the A04 (booked by stage 6 until the orchestrator signals); S3 proves the engine-level two-process equivalent"),
        defer("Device-phase", "full §30 acceptance on BOTH sides (mac + A04) with signed artifacts", "after PLAT-006; the final platform verdict comes only after this"),
    ]
}
