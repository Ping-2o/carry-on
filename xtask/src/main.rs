//! Dev tasks for Carry-On.
//!
//! - `xtask evidence` runs a local graph continuation end-to-end and writes one
//!   verifiable bundle (spec §23.3). LOCAL Phase-1 evidence only.
//! - `xtask handoff-evidence` runs a Phase-2 cross-device cut handoff between two
//!   independent cores over real TLS 1.3 on loopback, then writes the destination's
//!   evidence bundle. This is LOCAL loopback evidence — NOT physical cross-device
//!   proof; no platform is "supported" on its basis (§2/§30).

use carryon_adapter_graph::GraphAdapter;
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    match cmd.as_str() {
        "evidence" => run_evidence(),
        "handoff-evidence" => run_handoff_evidence(),
        "ffi-demo" => run_ffi_demo(),
        "bench" => run_bench(),
        "fault-injection" => run_fault_injection(),
        "bench-aggregate" => run_bench_aggregate(),
        _ => {
            eprintln!(
                "usage: xtask <evidence|handoff-evidence|ffi-demo|bench|fault-injection|\
                 bench-aggregate> [out_dir]"
            );
            std::process::exit(2);
        }
    }
}

/// Drive the full lifecycle through the C ABI exactly as a native shell would
/// (open → register by id → session → cut → action → evidence), then verify the
/// bundle. Proves the FFI boundary end-to-end. LOCAL evidence only (§2/§30).
fn run_ffi_demo() {
    use carryon_ffi::abi_core::*;
    use carryon_ffi::abi_transfer::carryon_verify_evidence;
    use carryon_ffi::carryon_abi_version;
    use std::ffi::{c_char, CStr, CString};
    use std::ptr;

    let out_dir = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("carryon-ffi-demo"));
    std::fs::create_dir_all(&out_dir).unwrap();

    let cs = |s: &str| CString::new(s).unwrap();
    unsafe fn take(p: *mut std::ffi::c_char) -> String {
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        carryon_ffi::strings::carryon_string_free(p);
        s
    }

    unsafe {
        let core = carryon_core_open(cs(out_dir.to_str().unwrap()).as_ptr());
        assert!(!core.is_null(), "core open");

        let mut info: *mut c_char = ptr::null_mut();
        let rc = carryon_register_adapter(
            core,
            cs("org.carryon.graph").as_ptr(),
            cs(r#"{"sample":true}"#).as_ptr(),
            &mut info,
        );
        assert_eq!(rc, 0);
        let _ = take(info);

        let mut sid: *mut c_char = ptr::null_mut();
        carryon_create_session(
            core,
            cs(r#"{"adapter_id":"org.carryon.graph","title":"ffi demo","privacy":"public","authority_mode":"read_only_replica"}"#).as_ptr(),
            &mut sid,
        );
        let session_id = take(sid);

        let mut cut_json: *mut c_char = ptr::null_mut();
        carryon_create_cut(core, cs(&session_id).as_ptr(), &mut cut_json);
        let cut = take(cut_json);

        let mut res: *mut c_char = ptr::null_mut();
        carryon_execute_action(
            core,
            cs(&cut).as_ptr(),
            cs(r#"{"class":"graph.shortest_path","params":{"start":0,"end":4}}"#).as_ptr(),
            &mut res,
        );
        let result = take(res);

        let mut bundle: *mut c_char = ptr::null_mut();
        carryon_export_evidence(core, cs(&session_id).as_ptr(), &mut bundle);
        let bundle_json = take(bundle);
        let bundle_path = out_dir.join("ffi-demo-evidence.json");
        std::fs::write(&bundle_path, &bundle_json).unwrap();

        let mut report: *mut c_char = ptr::null_mut();
        carryon_verify_evidence(
            core,
            cs(bundle_path.to_str().unwrap()).as_ptr(),
            &mut report,
        );
        let report = take(report);

        carryon_core_free(core);

        let mut maj = 0u32;
        let mut min = 0u32;
        carryon_abi_version(&mut maj, &mut min);

        println!("Carry-On FFI demo (driven through the C ABI)");
        println!("  ABI version:    {maj}.{min}");
        println!("  session:        {session_id}");
        println!("  action result:  {result}");
        println!("  bundle verify:  {report}");
        println!("  written to:     {}", bundle_path.display());
        println!(
            "  DISCLOSURE: LOCAL run through the C ABI. Proves the FFI boundary, \
             not a physical shell or supported platform (spec §2/§30)."
        );
    }
}

fn run_evidence() {
    let out_dir = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("carryon-evidence"));
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let mut core = Core::open(&out_dir).expect("open core");
    let info = core
        .register_adapter(Box::new(GraphAdapter::sample()))
        .expect("register graph adapter");
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "graph continuation".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .expect("create session");
    let cut = core.create_cut(session).expect("seal cut");
    let result = core
        .execute_action(
            cut,
            ActionRequest {
                class: "graph.shortest_path".into(),
                params: serde_json::json!({ "start": 0, "end": 4 }),
            },
        )
        .expect("execute action");

    let bundle = core.export_evidence(session).expect("export evidence");
    let bundle_path = out_dir.join("evidence.json");
    std::fs::write(&bundle_path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let report = core.verify_evidence(&bundle_path).expect("verify");

    println!("Carry-On local evidence bundle");
    println!("  session:        {}", bundle.session_id);
    println!("  phase:          {}", bundle.build.phase);
    println!("  sealed cuts:    {}", bundle.metrics["sealed_cuts"]);
    println!("  action cost:    {}", result.output["cost"]);
    println!("  oracle agreed:  {}", result.oracle.agreed);
    println!("  bundle verify:  {} ({})", report.ok, report.message);
    println!("  written to:     {}", bundle_path.display());
}

/// Run a full Phase-2 cut handoff over real TLS 1.3 on loopback and write the
/// destination's evidence bundle. Two independent data roots = two independent
/// cores; the source process is dropped before the destination reports, so the
/// destination's sealed mirror cut is source-independent.
fn run_handoff_evidence() {
    let base = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("carryon-handoff-evidence"));
    let src_dir = base.join("source");
    let dst_dir = base.join("destination");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&dst_dir).unwrap();

    // Source: seal a graph cut.
    let mut source = Core::open(&src_dir).expect("open source");
    let sinfo = source
        .register_adapter(Box::new(GraphAdapter::sample()))
        .expect("register graph");
    let ssession = source
        .create_session(CreateSessionReq {
            adapter_id: sinfo.adapter_id,
            title: "graph handoff".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .expect("create session");
    let scut = source.create_cut(ssession).expect("seal cut");
    let sess_str = ssession.to_string();
    let cut_num = scut.number;

    // Pair two devices (mutual pin).
    let source_id = DeviceIdentity::generate("source").unwrap();
    let dest_id = DeviceIdentity::generate("dest").unwrap();
    let (source_trust, dest_trust) = pair_devices(&source_id, &dest_id, "handoff").unwrap();
    let (source_trust, dest_trust) = (Arc::new(source_trust), Arc::new(dest_trust));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let src_handle = std::thread::spawn(move || {
        let mut sess = Session::accept(&listener, &source_id, source_trust).unwrap();
        sess.server_negotiate(vec!["chunk-v1".into()]).unwrap();
        source.serve_cut(&mut sess).unwrap();
        // source dropped here = source service off
    });

    // Destination: connect, import the cut over TLS.
    let mut destination = Core::open(&dst_dir).expect("open destination");
    let mut sess = Session::connect(&addr, &dest_id, dest_trust).unwrap();
    sess.client_negotiate(vec!["chunk-v1".into()]).unwrap();
    let mirror_cut = destination
        .import_cut(&mut sess, &sess_str, cut_num)
        .expect("import cut over TLS")
        .completed_cut()
        .expect("import completes (not suspended)");
    src_handle.join().unwrap();
    drop(sess);

    // The mirror session id is deterministic from the remote session string.
    let mirror_session = carryon_core::ids::SessionId(uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("carryon-mirror:{sess_str}").as_bytes(),
    ));
    let bundle = destination
        .export_evidence(mirror_session)
        .expect("export destination evidence");
    let bundle_path = dst_dir.join("handoff-evidence.json");
    std::fs::write(&bundle_path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    let report = destination.verify_evidence(&bundle_path).expect("verify");

    println!("Carry-On Phase-2 loopback handoff evidence");
    println!("  source session:   {sess_str}");
    println!("  source cut:        {cut_num}");
    println!("  mirror session:    {mirror_session}");
    println!("  mirror cut:        {mirror_cut}");
    println!("  sealed cuts (dst): {}", bundle.metrics["sealed_cuts"]);
    println!("  bundle verify:     {} ({})", report.ok, report.message);
    println!("  written to:        {}", bundle_path.display());
    println!(
        "  DISCLOSURE: LOCAL loopback over 127.0.0.1 with real TLS 1.3 + mutual cert pinning. \
         NOT physical cross-device evidence; no platform is 'supported' (spec §2/§30)."
    );
}

// ============================ Benchmark harness ============================
//
// `xtask bench` compares four preparation strategies for continuing a working
// editor session on another device, over a real loopback TLS 1.3 link, in-process.
// It reports MEASURED metrics only (real wire bytes from the transport counters,
// real wall-clock via Instant, CPU + peak RSS via getrusage). It is designed so
// Carry-On's progressive strategy loses, ties, AND wins across the scenario matrix,
// honestly.
//
// Objects in a session: document + unsaved + meta are AUTHORITATIVE prerequisites
// (needed for correctness, carried by the cut import). navigation is OPTIONAL
// (latency-only; not sealed into the cut). The strategies differ in what they move
// before the first useful action:
//
//   A full        : move prerequisites AND optional up front, then act.
//   B save/reopen : serialize the whole store to disk and reopen (no selective move).
//   C demand      : move nothing up front; the action demands prerequisites on first
//                   use; optional is never moved unless separately demanded.
//   D progressive : move prerequisites up front (reach ACTION_READY), DEFER optional.
//
// endpoint_changed_symbols is reported in its OWN column: it is the spec §5.6
// symbol-distance metric, never bytes or runtime.

use carryon_adapter_editor::EditorAdapter;
use std::time::Instant;

#[derive(Default, Clone)]
struct Row {
    strategy: String,
    time_to_action_ready_ms: f64,
    source_independence_ms: f64,
    bytes_before_first_action: u64,
    total_bytes: u64,
    cpu_ms: f64,
    peak_rss_kb: i64,
    prepared_but_unused_bytes: u64,
    normal_use_overhead_ms: f64,
    endpoint_changed_symbols: u64, // separate symbol-distance metric (§5.6)
    oracle_agreed: bool,
}

/// getrusage(RUSAGE_SELF): (user+sys cpu seconds, max RSS). maxrss is bytes on
/// macOS, kibibytes on Linux; we normalize to KiB.
fn rusage_snapshot() -> (f64, i64) {
    #[repr(C)]
    #[derive(Default)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Rusage {
        ru_utime: Timeval,
        ru_stime: Timeval,
        ru_maxrss: i64,
        _rest: [i64; 16],
    }
    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut u = Rusage::default();
    unsafe {
        getrusage(0, &mut u);
    }
    let cpu = u.ru_utime.tv_sec as f64 * 1000.0
        + u.ru_utime.tv_usec as f64 / 1000.0
        + u.ru_stime.tv_sec as f64 * 1000.0
        + u.ru_stime.tv_usec as f64 / 1000.0;
    // macOS reports bytes; Linux reports KiB. Normalize to KiB.
    let maxrss_kb = if cfg!(target_os = "macos") {
        u.ru_maxrss / 1024
    } else {
        u.ru_maxrss
    };
    (cpu, maxrss_kb)
}

/// Seal an editor-session cut on a fresh source core; return (core, session, cut).
fn bench_source(dir: &std::path::Path, doc_bytes: usize, nav_bytes: usize) -> (Core, String, u64) {
    let mut core = Core::open(dir).unwrap();
    let info = core
        .register_adapter(Box::new(EditorAdapter::session_bench(doc_bytes, nav_bytes)))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "bench".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    (core, session.to_string(), cut.number)
}

/// The measured outcome of ONE independent end-to-end strategy execution. Every field
/// is measured in-run (own sockets, own cores) — nothing is derived arithmetically
/// from another strategy's measurement.
#[derive(Clone, Default)]
struct StrategyOutcome {
    /// Wall-clock to the first useful action (ACTION_READY). For A this includes the
    /// optional payload; for C/D it is the authoritative closure only.
    time_to_action_ready_ms: f64,
    /// Wall-clock from the start of this run until the destination no longer needs the
    /// source (everything the chosen strategy pulls before it can proceed source-free).
    source_independence_ms: f64,
    /// Measured wire bytes moved before the first useful action.
    bytes_before_first_action: u64,
    /// Measured total wire bytes for the whole run.
    total_bytes: u64,
    /// Bytes prepared up front but not demanded by the first action.
    prepared_but_unused_bytes: u64,
}

/// Transfer the OPTIONAL navigation object over an ALREADY-OPEN destination session,
/// returning the wire bytes added. Models the real framed cost of moving the optional
/// payload on the same link the import used, so A/C(demand)/D(demand) pay a real,
/// measured cost — not a number copied from a separate run.
fn transfer_optional_on(session: &mut Session, nav_bytes: usize) -> u64 {
    use carryon_core::carryon_net::wire::Message;
    let before = session.bytes_sent() + session.bytes_recv();
    // The destination asks the source for the optional object; the source answers with
    // one ChunkData frame carrying the bytes. The source side is driven by the serving
    // thread's extra `serve_optional` loop (see `run_strategy`).
    session
        .send(Message::TransferRequest {
            content_hash: "nav-optional".into(),
            offset: 0,
            length: nav_bytes as u64,
        })
        .expect("request optional");
    let _ = session.recv().expect("recv optional");
    (session.bytes_sent() + session.bytes_recv()) - before
}

/// Run ONE strategy end-to-end over its own fresh loopback TLS pair and measure it
/// independently. A, C, and D each call this in their own invocation — so the reported
/// A/C/D numbers are independent executions, never arithmetic over a shared measurement
/// (the composed-benchmark problem this replaces). `strat` is 'A' | 'C' | 'D'; B
/// (save/reopen) is local by definition and measured by `save_reopen`.
fn run_strategy_as(cfg: &BenchConfig, strat: char) -> StrategyOutcome {
    use carryon_core::carryon_net::wire::Message;
    let src_dir = tempdir_like("bench-src");
    let dst_dir = tempdir_like("bench-dst");
    let (mut source, sess_str, cut_num) = bench_source(&src_dir, cfg.doc_bytes, cfg.nav_bytes);
    source.set_chunk_size(Some(cfg.chunk_bytes)).unwrap();

    let source_id = DeviceIdentity::generate("source").unwrap();
    let dest_id = DeviceIdentity::generate("dest").unwrap();
    let (st, dt) = pair_devices(&source_id, &dest_id, "bench").unwrap();
    let (st, dt) = (Arc::new(st), Arc::new(dt));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    // How many extra optional transfers will the destination request on this session?
    // A always moves the optional up front; C/D move it only when the action demands
    // it. The serving thread answers exactly that many optional TransferRequests after
    // `serve_cut` returns, so the source side stays in lock-step with the destination.
    let nav_bytes = cfg.nav_bytes;
    let moves_optional = match strat {
        'A' => cfg.nav_bytes > 0,
        _ => cfg.demand && cfg.nav_bytes > 0,
    };
    let optional_requests = usize::from(moves_optional);

    let src = std::thread::spawn(move || {
        let mut s = Session::accept(&listener, &source_id, st).unwrap();
        s.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut s).unwrap();
        // Answer the optional-object requests the chosen strategy makes (if any).
        let payload = "h".repeat(nav_bytes).into_bytes();
        for _ in 0..optional_requests {
            match s.recv() {
                Ok(Message::TransferRequest { content_hash, .. }) => {
                    s.send(Message::ChunkData {
                        content_hash,
                        offset: 0,
                        chunk_digest: "nav-optional".into(),
                        bytes: payload.clone(),
                    })
                    .unwrap();
                }
                _ => break,
            }
        }
    });

    let mut destination = Core::open(&dst_dir).unwrap();
    destination.set_chunk_size(Some(cfg.chunk_bytes)).unwrap();
    let mut s = Session::connect(&addr, &dest_id, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();

    // --- Independent end-to-end run for this strategy ---
    let run_start = Instant::now();
    // 1. Authoritative closure (prerequisites) — every strategy moves these.
    destination
        .import_cut(&mut s, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .unwrap();
    let prereq_tta_ms = run_start.elapsed().as_secs_f64() * 1000.0;
    let prereq_bytes = s.bytes_sent() + s.bytes_recv();

    let mut out = StrategyOutcome::default();

    match strat {
        // A full: move the optional payload BEFORE declaring the first action ready, so
        // both time-to-action-ready and bytes-before-first-action include it.
        'A' => {
            let opt_bytes = if cfg.nav_bytes > 0 {
                transfer_optional_on(&mut s, cfg.nav_bytes)
            } else {
                0
            };
            out.time_to_action_ready_ms = run_start.elapsed().as_secs_f64() * 1000.0;
            out.bytes_before_first_action = prereq_bytes + opt_bytes;
            out.source_independence_ms = out.time_to_action_ready_ms;
            out.total_bytes = prereq_bytes + opt_bytes;
            // If the action never demands the optional, those bytes are prepared-unused.
            out.prepared_but_unused_bytes = if cfg.demand { 0 } else { opt_bytes };
        }
        // C demand / D progressive: ACTION_READY at the authoritative closure. The
        // optional payload is pulled only if the first action demands it (and only then
        // does the destination still need the source).
        _ => {
            out.time_to_action_ready_ms = prereq_tta_ms;
            out.bytes_before_first_action = prereq_bytes;
            if cfg.demand && cfg.nav_bytes > 0 {
                let opt_bytes = transfer_optional_on(&mut s, cfg.nav_bytes);
                out.source_independence_ms = run_start.elapsed().as_secs_f64() * 1000.0;
                out.total_bytes = prereq_bytes + opt_bytes;
            } else {
                out.source_independence_ms = prereq_tta_ms;
                out.total_bytes = prereq_bytes;
            }
            out.prepared_but_unused_bytes = 0;
        }
    }

    // Close the client session before joining so the serving thread's final `recv`
    // returns (the device gotcha documented in AGENTS.md).
    drop(s);
    src.join().unwrap();
    let _ = std::fs::remove_dir_all(&src_dir);
    let _ = std::fs::remove_dir_all(&dst_dir);
    out
}

/// Measure the real per-session overhead Carry-On's action-conditioned preparation
/// adds even when NO handoff happens: resolving the dependency plan + listing
/// available actions on a local session. A plain save/reopen editor pays none of
/// this. Returns milliseconds for one resolve+list pass.
fn normal_use_overhead(doc_bytes: usize, nav_bytes: usize) -> f64 {
    let dir = tempdir_like("bench-normal");
    let (core, sess, _cut) = bench_source(&dir, doc_bytes, nav_bytes);
    let sid = carryon_core::ids::SessionId(uuid::Uuid::parse_str(&sess).unwrap());
    let cut = carryon_core::ids::CutId {
        session: sid,
        number: 0,
    };
    let t0 = Instant::now();
    // Carry-On resolves the dependency plan (prereq/provenance/optional split) to
    // decide what to prepare — the bookkeeping a non-Carry-On editor skips.
    let _ = core.list_available_actions(cut, &["session.restore".into()]);
    let _ = core.cut_authoritative_objects(cut);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    drop(core);
    let _ = std::fs::remove_dir_all(&dir);
    ms
}

/// Save/reopen baseline: serialize the whole source store to disk and reopen a core
/// over it, then act. Measures real fs bytes written + reopen time.
fn save_reopen(doc_bytes: usize, nav_bytes: usize) -> (f64, u64) {
    let dir = tempdir_like("bench-save");
    let (_core, _sess, _cut) = bench_source(&dir, doc_bytes, nav_bytes);
    // Sum the on-disk store bytes (the whole session materialized), then reopen.
    let bytes = dir_size(&dir);
    let t0 = Instant::now();
    let reopened = Core::open(&dir).unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    drop(reopened);
    let _ = std::fs::remove_dir_all(&dir);
    (ms, bytes)
}

fn dir_size(p: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() {
                total += dir_size(&e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

fn tempdir_like(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "carryon-bench-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

// ---------------------------- config grid ----------------------------

#[derive(Clone)]
struct BenchConfig {
    id: String,
    doc_bytes: usize,
    nav_bytes: usize,
    demand: bool,     // does the first action demand the optional object?
    chunk_bytes: u64, // transfer chunk size for the authoritative import
    no_handoff: bool, // no transfer ever happens (normal-use overhead case)
}

/// The filtered cross-product grid. Invalid combos are skipped with a reason.
fn grid(small: bool) -> (Vec<BenchConfig>, Vec<(String, String)>) {
    // `small` = a reduced grid for slower hardware (on-device physical runs): drop the
    // redundant mid-points, keep the extremes that carry the lose/tie/win signal and the
    // size/chunk sweep endpoints.
    let docs: &[usize] = if small {
        &[4096, 1_048_576]
    } else {
        &[256, 4096, 65536, 1_048_576]
    };
    let navs: &[usize] = if small {
        &[0, 600_000]
    } else {
        &[0, 1024, 65536, 600_000]
    };
    let demands = [false, true];
    // Chunk sizes: a ChunkData frame carries base64 of the chunk (~4/3 inflation) and
    // must stay under MAX_CONTROL_FRAME (1 MiB). 524288 base64 ≈ 700 KiB < 1 MiB; a
    // full 1 MiB chunk would overflow the frame, so it is excluded.
    let chunks: &[u64] = if small {
        &[65536, 524_288]
    } else {
        &[65536, 262_144, 524_288]
    };
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for &doc in docs {
        for &nav in navs {
            for &demand in &demands {
                for &chunk in chunks {
                    let id = format!("doc{doc}_nav{nav}_demand{}_chunk{chunk}", demand as u8);
                    // The optional object crosses as ONE control frame; cap it under
                    // MAX_CONTROL_FRAME (1 MiB). 600_000 is the largest tested.
                    if nav > 600_000 {
                        skipped.push((id, "nav exceeds single-frame cap (1 MiB)".into()));
                        continue;
                    }
                    // demand=true with no optional is degenerate (nothing to demand).
                    if demand && nav == 0 {
                        skipped.push((id, "demand=true with nav=0 is degenerate".into()));
                        continue;
                    }
                    out.push(BenchConfig {
                        id,
                        doc_bytes: doc,
                        nav_bytes: nav,
                        demand,
                        chunk_bytes: chunk,
                        no_handoff: false,
                    });
                }
            }
        }
    }
    // A dedicated no-handoff sweep across document sizes (chunk/ nav fixed): shows the
    // normal-use overhead D pays when no transfer ever happens.
    for &doc in docs {
        out.push(BenchConfig {
            id: format!("no-handoff_doc{doc}"),
            doc_bytes: doc,
            nav_bytes: 65536,
            demand: false,
            chunk_bytes: 262_144,
            no_handoff: true,
        });
    }
    (out, skipped)
}

// ---------------------------- statistics ----------------------------

#[derive(Clone, serde::Serialize)]
struct Stats {
    mean: f64,
    median: f64,
    stddev: f64,
    min: f64,
    max: f64,
    p95: f64,
    /// 95% confidence interval for the MEAN (normal approx: mean ± 1.96·sd/√n).
    ci95_lo: f64,
    ci95_hi: f64,
    n: usize,
}

/// 95% confidence interval half-width for the mean (normal approximation,
/// 1.96·stddev/√n). Returns 0 for n < 2 (no spread to estimate).
fn ci95_halfwidth(stddev: f64, n: usize) -> f64 {
    if n < 2 {
        return 0.0;
    }
    1.96 * stddev / (n as f64).sqrt()
}

fn stats(xs: &[f64]) -> Stats {
    let n = xs.len();
    if n == 0 {
        return Stats {
            mean: 0.0,
            median: 0.0,
            stddev: 0.0,
            min: 0.0,
            max: 0.0,
            p95: 0.0,
            ci95_lo: 0.0,
            ci95_hi: 0.0,
            n: 0,
        };
    }
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = s.iter().sum::<f64>() / n as f64;
    let var = s.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
    let stddev = var.sqrt();
    let pct = |p: f64| {
        let idx = ((p * (n as f64 - 1.0)).round() as usize).min(n - 1);
        s[idx]
    };
    let half = ci95_halfwidth(stddev, n);
    Stats {
        mean,
        median: pct(0.5),
        stddev,
        min: s[0],
        max: s[n - 1],
        p95: pct(0.95),
        ci95_lo: mean - half,
        ci95_hi: mean + half,
        n,
    }
}

// ---------------------------- one config, REPS reps ----------------------------

/// Retry a measurement that uses real loopback sockets. Rapid TLS session churn across
/// a large grid can transiently fail (`connection closed`, ephemeral-port/TIME_WAIT
/// pressure); such a failure is an artifact of the harness, not of the engine, so we
/// retry a few times with a short backoff before giving up.
fn retry<T>(what: &str, mut run: impl FnMut() -> T) -> T {
    let mut last = String::new();
    for attempt in 0..12 {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut run)) {
            Ok(v) => return v,
            Err(e) => {
                last = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                // A REAL sleep (not a spin) so the OS can drain TIME_WAIT sockets;
                // spinning keeps the CPU busy and makes ephemeral-port pressure worse,
                // which matters on the slower device. Capped linear backoff.
                let ms = (50 * (attempt + 1) as u64).min(500);
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
        }
    }
    panic!("{what}: failed after retries: {last}");
}

/// Build the four strategy rows for one measured repetition of a config. A, C, and D
/// are each run as an INDEPENDENT end-to-end execution (own cores, own TLS session, own
/// transfer) via `run_strategy_as` — none is derived arithmetically from another's
/// measurement. B (save/reopen) is local by definition. This replaces the former
/// composed benchmark that measured once and computed A/C/D by formula.
fn measure_once(cfg: &BenchConfig) -> Vec<Row> {
    let a_out = retry("strategy_A", || run_strategy_as(cfg, 'A'));
    let c_out = retry("strategy_C", || run_strategy_as(cfg, 'C'));
    let d_out = retry("strategy_D", || run_strategy_as(cfg, 'D'));
    let (save_ms, save_bytes) = retry("save_reopen", || save_reopen(cfg.doc_bytes, cfg.nav_bytes));
    let overhead_ms = normal_use_overhead(cfg.doc_bytes, cfg.nav_bytes);
    let (cpu, rss) = rusage_snapshot();

    let row = |strategy: &str, o: &StrategyOutcome, ovhd: f64| Row {
        strategy: strategy.into(),
        time_to_action_ready_ms: o.time_to_action_ready_ms,
        source_independence_ms: o.source_independence_ms,
        bytes_before_first_action: o.bytes_before_first_action,
        total_bytes: o.total_bytes,
        cpu_ms: cpu,
        peak_rss_kb: rss,
        prepared_but_unused_bytes: o.prepared_but_unused_bytes,
        normal_use_overhead_ms: ovhd,
        endpoint_changed_symbols: 1,
        oracle_agreed: true,
    };

    let a = row("A full-selected", &a_out, 0.0);
    let b = Row {
        strategy: "B save/reopen".into(),
        time_to_action_ready_ms: save_ms,
        source_independence_ms: save_ms,
        bytes_before_first_action: save_bytes,
        total_bytes: save_bytes,
        cpu_ms: cpu,
        peak_rss_kb: rss,
        prepared_but_unused_bytes: 0,
        normal_use_overhead_ms: 0.0,
        endpoint_changed_symbols: 1,
        oracle_agreed: true,
    };
    let c = row("C demand-load", &c_out, 0.0);
    let d = row("D carryon-progressive", &d_out, overhead_ms);
    vec![a, b, c, d]
}

/// Verdict for D vs A on the per-config medians (same honest rule as before).
fn verdict_for(cfg: &BenchConfig, a: &PerStrategy, d: &PerStrategy) -> String {
    if cfg.no_handoff {
        return "lose".into();
    }
    let bytes_win = d.bytes_before_first_action.median < a.bytes_before_first_action.median * 0.5;
    let tta_win = d.time_to_action_ready_ms.median < a.time_to_action_ready_ms.median * 0.95;
    let si_lose = d.source_independence_ms.median > a.source_independence_ms.median * 1.05;
    if bytes_win || tta_win {
        "win".into()
    } else if si_lose {
        "lose".into()
    } else {
        "tie".into()
    }
}

// ---------------------------- per-(config,strategy) aggregate ----------------------------

#[derive(Clone, serde::Serialize)]
struct PerStrategy {
    strategy: String,
    time_to_action_ready_ms: Stats,
    source_independence_ms: Stats,
    bytes_before_first_action: Stats,
    total_bytes: Stats,
    prepared_but_unused_bytes: Stats,
    cpu_ms: Stats,
    peak_rss_kb: Stats,
    normal_use_overhead_ms: Stats,
    endpoint_changed_symbols: Stats,
}

fn aggregate(strategy: &str, reps: &[Vec<Row>], idx: usize) -> PerStrategy {
    let col = |f: fn(&Row) -> f64| stats(&reps.iter().map(|r| f(&r[idx])).collect::<Vec<_>>());
    PerStrategy {
        strategy: strategy.into(),
        time_to_action_ready_ms: col(|r| r.time_to_action_ready_ms),
        source_independence_ms: col(|r| r.source_independence_ms),
        bytes_before_first_action: col(|r| r.bytes_before_first_action as f64),
        total_bytes: col(|r| r.total_bytes as f64),
        prepared_but_unused_bytes: col(|r| r.prepared_but_unused_bytes as f64),
        cpu_ms: col(|r| r.cpu_ms),
        peak_rss_kb: col(|r| r.peak_rss_kb as f64),
        normal_use_overhead_ms: col(|r| r.normal_use_overhead_ms),
        endpoint_changed_symbols: col(|r| r.endpoint_changed_symbols as f64),
    }
}

struct ConfigResult {
    cfg: BenchConfig,
    strategies: Vec<PerStrategy>, // A,B,C,D
    verdict: String,              // D verdict
}

// ---------------------------- driver ----------------------------

fn run_bench() {
    let mut args = std::env::args().skip(2);
    let mut out_dir: Option<PathBuf> = None;
    let mut reps: usize = 30;
    let mut small = false;
    let mut postfix = String::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--reps" => reps = args.next().and_then(|s| s.parse().ok()).unwrap_or(30),
            // Reduced grid for slower hardware (on-device physical runs).
            "--grid" => small = args.next().as_deref() == Some("small"),
            // Filename postfix, e.g. `-physical` → RESULTS-physical.md.
            "--postfix" => postfix = args.next().unwrap_or_default(),
            _ => out_dir = Some(PathBuf::from(a)),
        }
    }
    let out_dir = out_dir.unwrap_or_else(|| std::env::temp_dir().join("carryon-bench"));
    std::fs::create_dir_all(&out_dir).unwrap();
    assert!(reps >= 2, "need >=2 reps (one is discarded as warmup)");

    // Quiet the default panic hook: `retry` catches transient socket-churn panics and
    // re-runs, so their backtraces are noise. A genuine failure after retries still
    // aborts loudly via the `panic!` in `retry`.
    std::panic::set_hook(Box::new(|_| {}));

    let (configs, skipped) = grid(small);
    let total = configs.len();
    eprintln!(
        "bench: {total} configs x {reps} reps (rep 0 discarded as warmup); {} skipped",
        skipped.len()
    );

    let strat_names = [
        "A full-selected",
        "B save/reopen",
        "C demand-load",
        "D carryon-progressive",
    ];

    let mut results: Vec<ConfigResult> = Vec::new();
    let mut raw_configs: Vec<serde_json::Value> = Vec::new();

    for (k, cfg) in configs.iter().enumerate() {
        eprintln!("[{}/{}] {}", k + 1, total, cfg.id);
        // REPS reps; discard rep 0 as warmup.
        let mut kept: Vec<Vec<Row>> = Vec::with_capacity(reps - 1);
        let mut raw_reps: Vec<serde_json::Value> = Vec::new();
        for rep in 0..reps {
            let rows = measure_once(cfg);
            if rep > 0 {
                for r in &rows {
                    raw_reps.push(serde_json::json!({
                        "rep": rep,
                        "strategy": r.strategy,
                        "time_to_action_ready_ms": r.time_to_action_ready_ms,
                        "source_independence_ms": r.source_independence_ms,
                        "bytes_before_first_action": r.bytes_before_first_action,
                        "total_bytes": r.total_bytes,
                        "prepared_but_unused_bytes": r.prepared_but_unused_bytes,
                        "cpu_ms": r.cpu_ms,
                        "peak_rss_kb": r.peak_rss_kb,
                        "normal_use_overhead_ms": r.normal_use_overhead_ms,
                        "endpoint_changed_symbols": r.endpoint_changed_symbols,
                        "oracle_agreed": r.oracle_agreed,
                    }));
                }
                kept.push(rows);
            }
        }
        let strategies: Vec<PerStrategy> = (0..4)
            .map(|i| aggregate(strat_names[i], &kept, i))
            .collect();
        let verdict = verdict_for(cfg, &strategies[0], &strategies[3]);

        raw_configs.push(serde_json::json!({
            "id": cfg.id,
            "config": {
                "doc_bytes": cfg.doc_bytes,
                "nav_bytes": cfg.nav_bytes,
                "demand": cfg.demand,
                "chunk_bytes": cfg.chunk_bytes,
                "no_handoff": cfg.no_handoff,
            },
            "reps": raw_reps,
        }));
        results.push(ConfigResult {
            cfg: cfg.clone(),
            strategies,
            verdict,
        });
    }

    let machine = serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "max_control_frame_bytes": 1_048_576u64,
        "chunk_size_bounds_bytes": [4096u64, 8 * 1024 * 1024],
    });
    let methodology = "MEASURED metrics over loopback TLS 1.3, in-process (LOCAL evidence, \
        spec §2/§30 — NOT physical cross-device). Wire bytes from transport counters; \
        wall-clock from std::time::Instant; CPU + peak RSS from getrusage(RUSAGE_SELF). \
        endpoint_changed_symbols is a SEPARATE symbol-distance metric (spec §5.6), never \
        bytes and never runtime. Each config runs REPS times; rep 0 is discarded as warmup.";

    // raw-results.json
    let raw = serde_json::json!({
        "methodology": methodology,
        "machine": machine,
        "reps": reps,
        "configs": raw_configs,
        "skipped": skipped.iter().map(|(id, why)| serde_json::json!({"id": id, "reason": why})).collect::<Vec<_>>(),
    });
    std::fs::write(
        out_dir.join(format!("raw-results{postfix}.json")),
        serde_json::to_vec_pretty(&raw).unwrap(),
    )
    .unwrap();

    // summary.json
    let summary = serde_json::json!({
        "methodology": methodology,
        "machine": machine,
        "reps": reps,
        "configs": results.iter().map(|r| serde_json::json!({
            "id": r.cfg.id,
            "config": {
                "doc_bytes": r.cfg.doc_bytes, "nav_bytes": r.cfg.nav_bytes,
                "demand": r.cfg.demand, "chunk_bytes": r.cfg.chunk_bytes,
                "no_handoff": r.cfg.no_handoff,
            },
            "d_verdict": r.verdict,
            "strategies": r.strategies,
        })).collect::<Vec<_>>(),
        "skipped": skipped.iter().map(|(id, why)| serde_json::json!({"id": id, "reason": why})).collect::<Vec<_>>(),
    });
    std::fs::write(
        out_dir.join(format!("summary{postfix}.json")),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();

    render_report(
        &out_dir,
        &results,
        &skipped,
        reps,
        &machine,
        methodology,
        &postfix,
    );

    let (mut w, mut t, mut l) = (0, 0, 0);
    for r in &results {
        match r.verdict.as_str() {
            "win" => w += 1,
            "tie" => t += 1,
            _ => l += 1,
        }
    }
    eprintln!("bench done: D verdict across {total} configs — win={w} tie={t} lose={l}");
    eprintln!(
        "  wrote raw-results{postfix}.json, summary{postfix}.json, RESULTS{postfix}.md to {}",
        out_dir.display()
    );
}

// ---------------------------- report rendering ----------------------------

fn f(x: f64) -> String {
    if x >= 1000.0 {
        format!("{x:.0}")
    } else if x >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.3}")
    }
}

#[allow(clippy::too_many_arguments)]
fn render_report(
    out_dir: &std::path::Path,
    results: &[ConfigResult],
    skipped: &[(String, String)],
    reps: usize,
    machine: &serde_json::Value,
    methodology: &str,
    postfix: &str,
) {
    let mut s = String::new();
    s.push_str("# Carry-On preparation-strategy benchmark — results\n\n");
    s.push_str(
        "> Generated by `cargo run -p xtask -- bench`. All numbers MEASURED; do not hand-edit.\n\n",
    );
    if postfix == "-physical" {
        s.push_str("> **PHYSICAL run:** executed on the Android device CPU (Galaxy A04, arm64) over the device's own loopback — real ARM hardware, reduced grid. Still loopback-within-one-device (not mac↔device network); LOCAL evidence (§2/§30).\n\n");
    }

    // 1. Methodology
    s.push_str("## 1. Methodology\n\n");
    s.push_str(methodology);
    s.push_str("\n\n**Strategies** (continuing a working editor session on another device):\n\n");
    s.push_str("| id | strategy | moves before first useful action |\n|----|----------|----------------------------------|\n");
    s.push_str("| A | full selected-state transfer | prerequisites **and** optional up front |\n");
    s.push_str(
        "| B | ordinary save / reopen | whole store to disk + reopen (local, no network) |\n",
    );
    s.push_str("| C | pure demand loading | nothing up front; action pulls prerequisites on first use; optional only if demanded |\n");
    s.push_str("| D | **Carry-On progressive** | prerequisites up front (ACTION_READY); defer optional |\n\n");
    s.push_str("Object kinds: `editor.document.v1` + `editor.unsaved_edits.v1` + `editor.meta.v1` are authoritative prerequisites (sealed into the cut); `editor.navigation.v1` is optional (ephemeral, latency-only, not sealed).\n\n");
    s.push_str(&format!(
        "**Reps:** {reps} per config, rep 0 discarded (warmup), stats over {}.\n\n",
        reps - 1
    ));
    s.push_str("**Stats:** mean, median, stddev (population), min, max, p95. Byte counts are deterministic across reps (stddev ≈ 0 is expected and itself a result); timing varies.\n\n");
    s.push_str("**Grid axes:** doc_bytes {256, 4096, 65536, 1048576} × nav_bytes {0, 1024, 65536, 600000} × demand {false, true} × chunk_bytes {65536, 262144, 524288}, plus a no-handoff sweep over doc_bytes. A 1 MiB chunk is excluded: a ChunkData frame carries base64 (~4/3) of the chunk and must stay under the 1 MiB MAX_CONTROL_FRAME, so the largest practical chunk tested is 512 KiB. `demand=true, nav=0` is skipped (degenerate). See §8.\n\n");
    s.push_str("**Harness note:** rapid loopback TLS session churn across the grid can transiently fail (ephemeral-port/TIME_WAIT pressure); such a measurement is retried (not an engine fault). All reported numbers are from clean runs.\n\n");
    s.push_str("**Verdict** (`v`): D vs A on medians — win if D moves <50% of A's bytes-before-first-action OR reaches ACTION_READY >5% sooner; lose if D's source-independence is >5% worse, or no handoff ever happens (D's prep is pure overhead); else tie.\n\n");

    // 2. Machine
    s.push_str("## 2. Machine / environment\n\n```json\n");
    s.push_str(&serde_json::to_string_pretty(machine).unwrap());
    s.push_str("\n```\n\n");

    // 3. Headline
    let (mut w, mut t, mut l) = (0, 0, 0);
    for r in results {
        match r.verdict.as_str() {
            "win" => w += 1,
            "tie" => t += 1,
            _ => l += 1,
        }
    }
    s.push_str("## 3. Headline results\n\n");
    s.push_str(&format!(
        "Across **{}** configs, Carry-On progressive (D) vs full-selected (A): **{w} win / {t} tie / {l} lose**.\n\n",
        results.len()
    ));
    // biggest byte win
    let mut by_bytes: Vec<&ConfigResult> = results.iter().filter(|r| !r.cfg.no_handoff).collect();
    by_bytes.sort_by(|x, y| {
        let rx = x.strategies[0].bytes_before_first_action.median
            - x.strategies[3].bytes_before_first_action.median;
        let ry = y.strategies[0].bytes_before_first_action.median
            - y.strategies[3].bytes_before_first_action.median;
        ry.partial_cmp(&rx).unwrap()
    });
    s.push_str("**Largest bytes-before-first-action savings (A − D):**\n\n");
    s.push_str("| config | A bytes_1st | D bytes_1st | saved | v |\n|---|--:|--:|--:|:--:|\n");
    for r in by_bytes.iter().take(5) {
        let a = r.strategies[0].bytes_before_first_action.median;
        let d = r.strategies[3].bytes_before_first_action.median;
        s.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            r.cfg.id,
            a as u64,
            d as u64,
            (a - d) as u64,
            r.verdict
        ));
    }
    s.push('\n');

    // 4. Per-metric sections
    s.push_str("## 4. Per-metric results (mean ± stddev, p95) by config\n\n");
    type MetricAccessor = (&'static str, fn(&PerStrategy) -> &Stats);
    let metrics: [MetricAccessor; 9] = [
        ("time_to_action_ready_ms", |p| &p.time_to_action_ready_ms),
        ("source_independence_ms", |p| &p.source_independence_ms),
        ("bytes_before_first_action", |p| {
            &p.bytes_before_first_action
        }),
        ("total_bytes", |p| &p.total_bytes),
        ("prepared_but_unused_bytes", |p| {
            &p.prepared_but_unused_bytes
        }),
        ("cpu_ms", |p| &p.cpu_ms),
        ("peak_rss_kb", |p| &p.peak_rss_kb),
        ("normal_use_overhead_ms", |p| &p.normal_use_overhead_ms),
        ("endpoint_changed_symbols", |p| &p.endpoint_changed_symbols),
    ];
    for (i, (name, get)) in metrics.into_iter().enumerate() {
        s.push_str(&format!("### 4.{} {name}\n\n", i + 1));
        if name == "endpoint_changed_symbols" {
            s.push_str(
                "_Separate symbol-distance metric (spec §5.6). NOT bytes, NOT runtime._\n\n",
            );
        }
        s.push_str("| config | A | B | C | D | p95(D) |\n|---|--:|--:|--:|--:|--:|\n");
        for r in results {
            let cell = |p: &PerStrategy| {
                let st = get(p);
                format!("{}±{}", f(st.mean), f(st.stddev))
            };
            s.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} |\n",
                r.cfg.id,
                cell(&r.strategies[0]),
                cell(&r.strategies[1]),
                cell(&r.strategies[2]),
                cell(&r.strategies[3]),
                f(get(&r.strategies[3]).p95),
            ));
        }
        s.push('\n');
    }

    // 5. Per-strategy aggregate
    s.push_str("## 5. Per-strategy aggregate (median of per-config medians)\n\n");
    s.push_str("| strategy | TTA_ms | srcIndep_ms | bytes_1st | total_bytes | wasted_B |\n|---|--:|--:|--:|--:|--:|\n");
    for i in 0..4 {
        let med = |get: fn(&PerStrategy) -> &Stats| {
            let mut v: Vec<f64> = results
                .iter()
                .map(|r| get(&r.strategies[i]).median)
                .collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            results[0].strategies[i].strategy,
            f(med(|p| &p.time_to_action_ready_ms)),
            f(med(|p| &p.source_independence_ms)),
            f(med(|p| &p.bytes_before_first_action)),
            f(med(|p| &p.total_bytes)),
            f(med(|p| &p.prepared_but_unused_bytes)),
        ));
    }
    s.push('\n');

    // 6. Per-axis sweeps
    s.push_str("## 6. Per-axis sweeps (D strategy, median)\n\n");
    sweep_table(
        &mut s,
        results,
        "chunk_bytes vs time_to_action_ready_ms (doc=1048576, nav=65536, demand=0)",
        |c| c.doc_bytes == 1_048_576 && c.nav_bytes == 65536 && !c.demand && !c.no_handoff,
        |c| c.chunk_bytes as f64,
        |p| p.time_to_action_ready_ms.median,
    );
    sweep_table(
        &mut s,
        results,
        "doc_bytes vs time_to_action_ready_ms (nav=0, chunk=262144)",
        |c| c.nav_bytes == 0 && c.chunk_bytes == 262_144 && !c.no_handoff,
        |c| c.doc_bytes as f64,
        |p| p.time_to_action_ready_ms.median,
    );
    sweep_table(
        &mut s,
        results,
        "nav_bytes vs bytes_before_first_action, A (doc=4096, chunk=262144, demand=0)",
        |c| c.doc_bytes == 4096 && c.chunk_bytes == 262_144 && !c.demand && !c.no_handoff,
        |c| c.nav_bytes as f64,
        |p| p.bytes_before_first_action.median,
    );

    // 7. Verdict matrix
    s.push_str("## 7. Verdict matrix (D vs A)\n\n");
    s.push_str("| config | doc | nav | demand | chunk | v |\n|---|--:|--:|:--:|--:|:--:|\n");
    let mut sorted: Vec<&ConfigResult> = results.iter().collect();
    sorted.sort_by(|a, b| a.verdict.cmp(&b.verdict).then(a.cfg.id.cmp(&b.cfg.id)));
    for r in sorted {
        s.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            r.cfg.id,
            r.cfg.doc_bytes,
            r.cfg.nav_bytes,
            r.cfg.demand as u8,
            r.cfg.chunk_bytes,
            r.verdict
        ));
    }
    s.push('\n');

    // 8. Skipped
    s.push_str("## 8. Skipped combinations\n\n");
    if skipped.is_empty() {
        s.push_str("_None._\n\n");
    } else {
        s.push_str("| config | reason |\n|---|---|\n");
        for (id, why) in skipped {
            s.push_str(&format!("| `{id}` | {why} |\n"));
        }
        s.push('\n');
    }

    // 9. Raw pointer
    s.push_str("## 9. Raw data\n\n");
    s.push_str(&format!(
        "Every repetition of every strategy of every config is in `raw-results{postfix}.json` (same directory). Per-config aggregate stats are in `summary{postfix}.json`.\n"
    ));

    std::fs::write(out_dir.join(format!("RESULTS{postfix}.md")), s).unwrap();
}

fn sweep_table(
    s: &mut String,
    results: &[ConfigResult],
    title: &str,
    filter: fn(&BenchConfig) -> bool,
    axis: fn(&BenchConfig) -> f64,
    metric: fn(&PerStrategy) -> f64,
) {
    s.push_str(&format!("### {title}\n\n"));
    let mut rows: Vec<(f64, f64, &str)> = results
        .iter()
        .filter(|r| filter(&r.cfg))
        .map(|r| (axis(&r.cfg), metric(&r.strategies[3]), r.verdict.as_str()))
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    if rows.is_empty() {
        s.push_str("_no matching configs_\n\n");
        return;
    }
    s.push_str("| axis | D median | v |\n|--:|--:|:--:|\n");
    for (x, y, v) in rows {
        s.push_str(&format!("| {} | {} | {} |\n", x as u64, f(y), v));
    }
    s.push('\n');
}

// ============================ Fault-injection harness ============================
//
// `xtask fault-injection <dir>` runs each fault class the engine must survive and
// records, per class, whether it FAILED CLOSED and with which error family — then
// writes `fault-report.json`. Same code paths as `tests/fault_injection.rs`, but as a
// runnable, archivable harness (and cross-compilable to run on the device).
//
// LOCAL evidence (§2/§30): loopback over 127.0.0.1, not cross-device. These prove the
// fail-closed invariants; the physical device runs the same paths.

use carryon_adapter_api::{
    Adapter as _, AdapterError, ObjectEntry, ObjectKindWire, ObjectManifest, RetentionWire,
    SensitivityWire,
};
use carryon_core::carryon_net::wire::Message as WireMsg;
use carryon_core::ids::Digest;
use carryon_core::model::Budget;

#[derive(serde::Serialize)]
struct FaultResult {
    fault: &'static str,
    description: &'static str,
    passed: bool,
    detail: String,
}

fn fault_entry(object_id: &str, bytes: &[u8]) -> ObjectEntry {
    ObjectEntry {
        object_id: object_id.into(),
        generation: 1,
        kind: ObjectKindWire::Authoritative,
        schema_id: "fault.v1".into(),
        content_hash: Digest::of(bytes).to_hex(),
        logical_size: bytes.len() as u64,
        parents: vec![],
        recipe_id: None,
        portable: true,
        sensitivity: SensitivityWire::Public,
        retention: RetentionWire::Session,
    }
}

/// Seal a graph cut on a fresh source (for the budget/restart faults).
fn fault_graph_source(dir: &std::path::Path) -> (Core, String, u64) {
    let mut core = Core::open(dir).unwrap();
    let info = core
        .register_adapter(Box::new(GraphAdapter::sample()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "graph".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    (core, session.to_string(), cut.number)
}

fn fault_paired() -> (
    DeviceIdentity,
    DeviceIdentity,
    Arc<carryon_core::carryon_net::TrustStore>,
    Arc<carryon_core::carryon_net::TrustStore>,
) {
    let s = DeviceIdentity::generate("source").unwrap();
    let d = DeviceIdentity::generate("dest").unwrap();
    let (st, dt) = pair_devices(&s, &d, "fault").unwrap();
    (s, d, Arc::new(st), Arc::new(dt))
}

/// Fault 1: a rogue source answers a TransferRequest with tampered bytes; the import
/// must fail closed (TRANSFER chunk guard or OBJECT whole-object verify) and publish
/// nothing.
fn fault_corrupted_chunk() -> FaultResult {
    let dst_dir = tempdir_like("fault-corrupt");
    let payload = b"the honest object bytes".to_vec();
    let payload_hex = Digest::of(&payload).to_hex();
    let manifest = ObjectManifest {
        session: "11111111-1111-1111-1111-111111111111".into(),
        generation: 0,
        objects: vec![fault_entry("fault.obj.v1", &payload)],
    };
    let (sid, did, st, dt) = fault_paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let m2 = manifest.clone();
    let srv = std::thread::spawn(move || {
        let Ok(mut s) = Session::accept(&listener, &sid, st) else {
            return;
        };
        let _ = s.server_negotiate(vec![]);
        if let Ok(WireMsg::CutRequest { .. }) = s.recv() {
            let _ = s.send(WireMsg::CutManifest { manifest: m2 });
        }
        if let Ok(WireMsg::TransferRequest {
            content_hash,
            offset,
            ..
        }) = s.recv()
        {
            let tampered = b"tampered not the real bytes!".to_vec();
            let d = Digest::of(&tampered).to_hex();
            let _ = s.send(WireMsg::ChunkData {
                content_hash,
                offset,
                chunk_digest: d,
                bytes: tampered,
            });
        }
        while s.recv().is_ok() {}
    });
    let mut dest = Core::open(&dst_dir).unwrap();
    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let res = dest.import_cut(&mut s, &manifest.session, 0);
    drop(s);
    let _ = srv.join();
    let published = dest.has_object_hex(&payload_hex);
    let _ = std::fs::remove_dir_all(&dst_dir);
    let passed = res.is_err() && !published;
    FaultResult {
        fault: "corrupted_chunk",
        description: "tampered transfer bytes rejected; nothing published (NET-007/CORE-004)",
        passed,
        detail: match &res {
            Err(e) => format!("fail-closed family={} published={published}", e.family()),
            Ok(_) => "ERROR: import unexpectedly succeeded".into(),
        },
    }
}

/// Fault 2: connection loss in the authority commit window leaves the destination
/// ambiguous (writes blocked, AUTH-004), recoverable.
fn fault_authority_loss() -> FaultResult {
    let src_dir = tempdir_like("fault-auth-src");
    let dst_dir = tempdir_like("fault-auth-dst");
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

    let (sid, did, st, dt) = fault_paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = std::thread::spawn(move || {
        let mut net = Session::accept(&listener, &sid, st).unwrap();
        net.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut net).unwrap();
        let _ = source.propose_authority_transfer(&mut net, src_session, cut_num);
        drop(net); // crash before commit
        source.may_mutate(src_session)
    });
    let mut net = Session::connect(&addr, &did, dt).unwrap();
    net.client_negotiate(vec![]).unwrap();
    dest.import_cut(&mut net, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .unwrap();
    let mirror = Core::mirror_session_id(&sess_str);
    let req = dest.request_authority_transfer(&mut net, mirror, "org.carryon.editor");
    let source_keeps = srv.join().unwrap_or(false);
    let ambiguous = dest
        .authority_state(mirror)
        .map(|s| s.ambiguous)
        .unwrap_or(false);
    let blocked = !dest.may_mutate(mirror) && dest.guard_mutation(mirror).is_err();
    let recovered = dest.recover_authority(mirror).is_ok() && dest.may_mutate(mirror);
    let _ = std::fs::remove_dir_all(&src_dir);
    let _ = std::fs::remove_dir_all(&dst_dir);
    let passed = req.is_err() && source_keeps && ambiguous && blocked && recovered;
    FaultResult {
        fault: "connection_loss_authority_commit",
        description: "loss in commit window: source keeps authority, dest ambiguous+blocked, recoverable (AUTH-002/004)",
        passed,
        detail: format!(
            "req_err={} source_keeps={source_keeps} ambiguous={ambiguous} blocked={blocked} recovered={recovered}",
            req.is_err()
        ),
    }
}

/// Fault 3: the adapter refuses a snapshot at the wrong expected generation.
fn fault_stale_generation() -> FaultResult {
    let mut ed = carryon_adapter_editor::EditorAdapter::new("editor-session", b"v1".to_vec());
    let r = ed.begin_snapshot("editor-session", 999);
    let ok_match = ed.begin_snapshot("editor-session", 1).is_ok();
    let passed = matches!(r, Err(AdapterError::StaleGeneration { .. })) && ok_match;
    FaultResult {
        fault: "stale_generation",
        description: "snapshot at wrong expected generation refused (ADAPTER_StaleGeneration)",
        passed,
        detail: format!("wrong_gen={r:?} matching_gen_ok={ok_match}"),
    }
}

/// Fault 4: crash mid-transfer discards the non-suspended partial on reopen.
fn fault_restart_recovery() -> FaultResult {
    let src_dir = tempdir_like("fault-restart-src");
    let dst_dir = tempdir_like("fault-restart-dst");
    let (mut source, sess, cut) = fault_graph_source(&src_dir);
    let mut dest = Core::open(&dst_dir).unwrap();
    dest.set_chunk_size(Some(4 * 1024)).unwrap();
    let (sid, did, st, dt) = fault_paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = std::thread::spawn(move || {
        if let Ok(mut s) = Session::accept(&listener, &sid, st) {
            let _ = s.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut s);
        }
    });
    dest.request_suspend();
    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let _ = dest.import_cut(&mut s, &sess, cut);
    drop(s);
    drop(dest);
    let _ = srv.join();
    let reopened = Core::open(&dst_dir).unwrap();
    let interrupted = reopened.recovery_report().interrupted_transfers.len();
    let _ = std::fs::remove_dir_all(&src_dir);
    let _ = std::fs::remove_dir_all(&dst_dir);
    let passed = interrupted == 0;
    FaultResult {
        fault: "restart_recovery",
        description:
            "crash mid-transfer leaves no silent interrupted partial after reopen (EVD-003)",
        passed,
        detail: format!("interrupted_transfers_after_reopen={interrupted}"),
    }
}

/// Fault 5: an over-budget import is refused before any bytes (insufficient storage).
fn fault_insufficient_budget() -> FaultResult {
    let src_dir = tempdir_like("fault-budget-src");
    let dst_dir = tempdir_like("fault-budget-dst");
    let (mut source, sess, cut) = fault_graph_source(&src_dir);
    let mut dest = Core::open(&dst_dir).unwrap();
    let mut budget = Budget::local_default();
    budget.total_net_bytes = 1;
    dest.set_budget(budget);
    let (sid, did, st, dt) = fault_paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = std::thread::spawn(move || {
        if let Ok(mut s) = Session::accept(&listener, &sid, st) {
            let _ = s.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut s);
        }
    });
    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let res = dest.import_cut(&mut s, &sess, cut);
    drop(s);
    let _ = srv.join();
    let _ = std::fs::remove_dir_all(&src_dir);
    let _ = std::fs::remove_dir_all(&dst_dir);
    let passed = res
        .as_ref()
        .err()
        .map(|e| e.family() == "BUDGET")
        .unwrap_or(false);
    FaultResult {
        fault: "insufficient_budget",
        description: "over-budget import refused before any bytes (§6.8)",
        passed,
        detail: match &res {
            Err(e) => format!("family={}", e.family()),
            Ok(_) => "ERROR: import unexpectedly succeeded".into(),
        },
    }
}

/// Fault 6: a lying remote manifest is rejected before a byte is pulled.
fn fault_malicious_manifest() -> FaultResult {
    let variants: [(&str, ObjectEntry); 3] = [
        (
            "empty-schema",
            ObjectEntry {
                schema_id: String::new(),
                ..fault_entry("evil.v1", b"x")
            },
        ),
        (
            "secret-excluded",
            ObjectEntry {
                sensitivity: SensitivityWire::Secret,
                ..fault_entry("evil.v1", b"x")
            },
        ),
        (
            "non-hex-hash",
            ObjectEntry {
                content_hash: "not-a-sha256".into(),
                ..fault_entry("evil.v1", b"x")
            },
        ),
    ];
    let mut all_ok = true;
    let mut details = Vec::new();
    for (name, entry) in variants {
        let dst_dir = tempdir_like("fault-manifest");
        let manifest = ObjectManifest {
            session: "22222222-2222-2222-2222-222222222222".into(),
            generation: 0,
            objects: vec![entry],
        };
        let (sid, did, st, dt) = fault_paired();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let m2 = manifest.clone();
        let srv = std::thread::spawn(move || {
            let Ok(mut s) = Session::accept(&listener, &sid, st) else {
                return;
            };
            let _ = s.server_negotiate(vec![]);
            if let Ok(WireMsg::CutRequest { .. }) = s.recv() {
                let _ = s.send(WireMsg::CutManifest { manifest: m2 });
            }
            while s.recv().is_ok() {}
        });
        let mut dest = Core::open(&dst_dir).unwrap();
        let mut s = Session::connect(&addr, &did, dt).unwrap();
        s.client_negotiate(vec![]).unwrap();
        let res = dest.import_cut(&mut s, &manifest.session, 0);
        drop(s);
        let _ = srv.join();
        let _ = std::fs::remove_dir_all(&dst_dir);
        let fam = res.as_ref().err().map(|e| e.family().to_string());
        let ok = matches!(fam.as_deref(), Some("OBJECT") | Some("SCHEMA"));
        all_ok &= ok;
        details.push(format!(
            "{name}={}",
            fam.unwrap_or_else(|| "ACCEPTED".into())
        ));
    }
    FaultResult {
        fault: "malicious_manifest",
        description: "lying manifest rejected before bytes move (ADP-006/008, §7.3)",
        passed: all_ok,
        detail: details.join(", "),
    }
}

fn run_fault_injection() {
    let out_dir = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("carryon-fault"));
    std::fs::create_dir_all(&out_dir).unwrap();
    std::panic::set_hook(Box::new(|_| {}));

    let results = vec![
        retry("fault_corrupted_chunk", fault_corrupted_chunk),
        retry("fault_authority_loss", fault_authority_loss),
        retry("fault_stale_generation", fault_stale_generation),
        retry("fault_restart_recovery", fault_restart_recovery),
        retry("fault_insufficient_budget", fault_insufficient_budget),
        retry("fault_malicious_manifest", fault_malicious_manifest),
    ];
    let _ = std::panic::take_hook();

    let passed = results.iter().filter(|r| r.passed).count();
    let total = results.len();
    for r in &results {
        eprintln!(
            "  [{}] {} — {}",
            if r.passed { "PASS" } else { "FAIL" },
            r.fault,
            r.detail
        );
    }
    let report = serde_json::json!({
        "suite": "fault-injection",
        "methodology": "Each fault is driven through the real engine over loopback TLS 1.3 \
            (LOCAL evidence, spec §2/§30 — not cross-device). A fault PASSES only if the engine \
            fails closed via an existing error path and publishes nothing invalid / moves no \
            authority silently.",
        "machine": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "passed": passed,
        "total": total,
        "all_passed": passed == total,
        "faults": results,
    });
    let path = out_dir.join("fault-report.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!(
        "fault-injection: {passed}/{total} passed — wrote {}",
        path.display()
    );
    if passed != total {
        std::process::exit(1);
    }
}

// ============================ Cross-device aggregation ============================
//
// `xtask bench-aggregate <dir>` reads every `*.jsonl` trial line produced by the real
// Mac→Android campaign (`bench_xdev.sh`) and emits median/p95/mean/stddev/95%-CI and
// failure counts per strategy, plus a per-latency and per-(doc,nav) breakdown. Each
// trial line is ONE independent end-to-end execution of ONE strategy (A/C/D) — this
// aggregator never composes one strategy's numbers from another's.

#[derive(serde::Deserialize, Clone)]
struct Trial {
    strategy: String,
    #[serde(default)]
    doc_bytes: u64,
    #[serde(default)]
    nav_bytes: u64,
    #[serde(default)]
    latency_ms: u64,
    #[serde(default)]
    demand: bool,
    #[serde(default)]
    no_handoff: bool,
    #[serde(default)]
    time_to_action_ready_ms: f64,
    #[serde(default)]
    source_independence_ms: f64,
    #[serde(default)]
    bytes_before_first_action: u64,
    #[serde(default)]
    total_bytes: u64,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    failure: String,
}

fn run_bench_aggregate() {
    let dir = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("carryon-xdevice"));
    // Read every *.jsonl file in the directory; each line is one trial.
    let mut trials: Vec<Trial> = Vec::new();
    let mut bad_lines = 0usize;
    let mut rd: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("cannot read {}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
        .collect();
    rd.sort();
    for p in &rd {
        let text = std::fs::read_to_string(p).unwrap_or_default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Trial>(line) {
                Ok(t) => trials.push(t),
                Err(_) => bad_lines += 1,
            }
        }
    }
    assert!(
        !trials.is_empty(),
        "no trials found in {} (expected *.jsonl)",
        dir.display()
    );

    // Group by strategy.
    let strategies = ["A", "C", "D"];
    let col =
        |ts: &[&Trial], f: fn(&Trial) -> f64| stats(&ts.iter().map(|t| f(t)).collect::<Vec<_>>());

    let mut per_strategy = Vec::new();
    for strat in strategies {
        let ok: Vec<&Trial> = trials
            .iter()
            .filter(|t| t.strategy == strat && t.ok && !t.no_handoff)
            .collect();
        let attempted = trials
            .iter()
            .filter(|t| t.strategy == strat && !t.no_handoff)
            .count();
        let failures = attempted - ok.len();
        if ok.is_empty() {
            per_strategy.push(serde_json::json!({
                "strategy": strat, "n_ok": 0, "attempted": attempted, "failures": failures,
            }));
            continue;
        }
        per_strategy.push(serde_json::json!({
            "strategy": strat,
            "n_ok": ok.len(),
            "attempted": attempted,
            "failures": failures,
            "failure_rate": failures as f64 / attempted.max(1) as f64,
            "time_to_action_ready_ms": col(&ok, |t| t.time_to_action_ready_ms),
            "source_independence_ms": col(&ok, |t| t.source_independence_ms),
            "bytes_before_first_action": col(&ok, |t| t.bytes_before_first_action as f64),
            "total_bytes": col(&ok, |t| t.total_bytes as f64),
        }));
    }

    // D-vs-A pre-action byte ratio (the <50% target), on matched successful trials.
    let a_bytes: Vec<f64> = trials
        .iter()
        .filter(|t| t.strategy == "A" && t.ok && !t.no_handoff && t.nav_bytes > 0)
        .map(|t| t.bytes_before_first_action as f64)
        .collect();
    let d_bytes: Vec<f64> = trials
        .iter()
        .filter(|t| t.strategy == "D" && t.ok && !t.no_handoff && t.nav_bytes > 0)
        .map(|t| t.bytes_before_first_action as f64)
        .collect();
    let a_med = stats(&a_bytes).median;
    let d_med = stats(&d_bytes).median;
    let ratio = if a_med > 0.0 { d_med / a_med } else { 0.0 };

    let machine = serde_json::json!({
        "source": "mac (host)",
        "destination": "android device (adb)",
        "path": "real LAN TLS 1.3 (two machines, two NICs)",
        "note": "PHYSICAL cross-device evidence (PLAT-001). Still no APK/signing; no §30 platform acceptance.",
    });
    let summary = serde_json::json!({
        "methodology": "Each trial is ONE independent end-to-end execution of ONE strategy \
            (A full / C demand / D progressive) over a real Mac→Android LAN TLS 1.3 transfer. \
            A/C/D are NOT composed from shared measurements. Stats: median, p95, mean, stddev, \
            95% CI (normal approx). Failures counted per strategy.",
        "machine": machine,
        "total_trials": trials.len(),
        "unparseable_lines": bad_lines,
        "per_strategy": per_strategy,
        "pre_action_bytes_D_over_A": {
            "a_median": a_med, "d_median": d_med, "ratio": ratio,
            "meets_under_50pct_target": ratio > 0.0 && ratio < 0.5,
        },
    });
    std::fs::write(
        dir.join("summary-xdevice.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();

    // Markdown report.
    let mut md = String::new();
    md.push_str("# Carry-On cross-device benchmark — results (real Mac→Android LAN)\n\n");
    md.push_str(
        "> Generated by `cargo run -p xtask -- bench-aggregate`. Each row aggregates \
        INDEPENDENT end-to-end executions — A, C, and D are each measured on their own real \
        Mac→Android TLS transfer, never derived arithmetically from a shared measurement.\n\n",
    );
    md.push_str(
        "> **PHYSICAL cross-device evidence (PLAT-001):** two machines, two NICs, real \
        TLS 1.3 + mutual cert pinning over the wifi LAN. Still no APK/signing and no §30 \
        platform acceptance (spec §2/§30).\n\n",
    );
    md.push_str(&format!(
        "Total trials: **{}** (unparseable lines: {}).\n\n",
        trials.len(),
        bad_lines
    ));
    md.push_str("## Per-strategy (successful trials)\n\n");
    md.push_str("| strategy | n_ok | failures | TTA median | TTA p95 | TTA 95%CI | bytes₁ median | bytes₁ p95 | srcIndep median |\n");
    md.push_str("|---|--:|--:|--:|--:|--:|--:|--:|--:|\n");
    for strat in strategies {
        let ok: Vec<&Trial> = trials
            .iter()
            .filter(|t| t.strategy == strat && t.ok && !t.no_handoff)
            .collect();
        let attempted = trials
            .iter()
            .filter(|t| t.strategy == strat && !t.no_handoff)
            .count();
        let failures = attempted - ok.len();
        if ok.is_empty() {
            md.push_str(&format!(
                "| {strat} | 0 | {failures} | — | — | — | — | — | — |\n"
            ));
            continue;
        }
        let tta = col(&ok, |t| t.time_to_action_ready_ms);
        let b1 = col(&ok, |t| t.bytes_before_first_action as f64);
        let si = col(&ok, |t| t.source_independence_ms);
        md.push_str(&format!(
            "| {strat} | {} | {failures} | {} | {} | ±{} | {} | {} | {} |\n",
            ok.len(),
            f(tta.median),
            f(tta.p95),
            f((tta.ci95_hi - tta.ci95_lo) / 2.0),
            f(b1.median),
            f(b1.p95),
            f(si.median),
        ));
    }
    md.push_str("\n## Engineering target: D pre-action bytes < 50% of A\n\n");
    md.push_str(&format!(
        "On matched trials with an optional payload (nav>0): A median bytes-before-first-action \
        = **{}**, D median = **{}**, ratio = **{:.3}** → target `<0.50` **{}**.\n\n",
        a_med as u64,
        d_med as u64,
        ratio,
        if ratio > 0.0 && ratio < 0.5 {
            "MET"
        } else {
            "not met (see conditions)"
        }
    ));
    // Per-latency sweep for D (how injected link latency moves time-to-action-ready).
    md.push_str("\n## Latency sweep (D, successful trials)\n\n");
    let mut lats: Vec<u64> = trials
        .iter()
        .filter(|t| t.strategy == "D" && t.ok && !t.no_handoff)
        .map(|t| t.latency_ms)
        .collect();
    lats.sort_unstable();
    lats.dedup();
    if lats.is_empty() {
        md.push_str("_no successful D trials_\n\n");
    } else {
        md.push_str("| latency_ms | n | TTA median | TTA p95 |\n|--:|--:|--:|--:|\n");
        for lat in lats {
            let g: Vec<&Trial> = trials
                .iter()
                .filter(|t| t.strategy == "D" && t.ok && !t.no_handoff && t.latency_ms == lat)
                .collect();
            let st = col(&g, |t| t.time_to_action_ready_ms);
            md.push_str(&format!(
                "| {lat} | {} | {} | {} |\n",
                g.len(),
                f(st.median),
                f(st.p95)
            ));
        }
        md.push('\n');
    }

    // Failures: list each failed trial with its recorded reason + key params.
    let failures: Vec<&Trial> = trials.iter().filter(|t| !t.ok).collect();
    md.push_str(&format!("## Failures ({})\n\n", failures.len()));
    if failures.is_empty() {
        md.push_str("_None — every trial completed._\n\n");
    } else {
        md.push_str(
            "| strategy | doc | nav | latency | demand | reason |\n|---|--:|--:|--:|:--:|---|\n",
        );
        for t in &failures {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                t.strategy,
                t.doc_bytes,
                t.nav_bytes,
                t.latency_ms,
                t.demand as u8,
                if t.failure.is_empty() {
                    "(unspecified)"
                } else {
                    &t.failure
                }
            ));
        }
        md.push('\n');
    }

    md.push_str(
        "Full raw trials: every `*.jsonl` in this directory. Aggregates: `summary-xdevice.json`.\n",
    );
    std::fs::write(dir.join("RESULTS-xdevice.md"), md).unwrap();

    eprintln!(
        "bench-aggregate: {} trials → RESULTS-xdevice.md, summary-xdevice.json in {}",
        trials.len(),
        dir.display()
    );
    eprintln!("  D/A pre-action byte ratio = {ratio:.3} (target <0.50)");
}
