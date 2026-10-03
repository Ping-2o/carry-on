//! Phase 4 capstone: a full **L4 single-writer authority transfer** between two
//! independent cores over real TLS 1.3, driven in one process over loopback, plus
//! its failure and recovery paths (spec §21.2/§21.3, AUTH-001..005; acceptance
//! §26.3 "crash and split-brain recovery tests").
//!
//! LOCAL EVIDENCE ONLY (spec §2/§30). This proves the end-to-end authority path —
//! import → propose → accept → commit → relinquish — and that a crash in the
//! ambiguous window blocks writes. It is **not** physical cross-device evidence; no
//! platform is "supported" on its basis.

use carryon_adapter_editor::EditorAdapter;
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session, TrustStore};
use carryon_core::ids::SessionId;
use carryon_core::model::{AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

const EDITOR_ID: &str = "org.carryon.editor";
const DRAFT: &[u8] = b"cooperative draft v1";

/// Seal an editor cut on a fresh single-writer source core.
fn seal_editor_source(dir: &std::path::Path) -> (Core, SessionId, u64) {
    let mut core = Core::open(dir).unwrap();
    let info = core
        .register_adapter(Box::new(EditorAdapter::new(
            "editor-session",
            DRAFT.to_vec(),
        )))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "editor".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    (core, session, cut.number)
}

/// A destination core with the editor adapter registered to continue the mirror.
fn dest_core(dir: &std::path::Path) -> Core {
    let mut core = Core::open(dir).unwrap();
    core.register_adapter(Box::new(EditorAdapter::new("mirror", DRAFT.to_vec())))
        .unwrap();
    core
}

type Paired = (
    DeviceIdentity,
    DeviceIdentity,
    Arc<TrustStore>,
    Arc<TrustStore>,
    TcpListener,
    String,
);

fn paired_loopback() -> Paired {
    let source_id = DeviceIdentity::generate("source").unwrap();
    let dest_id = DeviceIdentity::generate("dest").unwrap();
    let (st, dt) = pair_devices(&source_id, &dest_id, "t").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    (
        source_id,
        dest_id,
        Arc::new(st),
        Arc::new(dt),
        listener,
        addr,
    )
}

#[test]
fn full_l4_authority_transfer_moves_ownership() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, src_session, cut_num) = seal_editor_source(src_dir.path());
    let mut destination = dest_core(dst_dir.path());
    let sess_str = src_session.to_string();

    let (source_id, dest_id, st, dt, listener, addr) = paired_loopback();

    // Source thread: serve the cut, then propose + relinquish authority.
    let src_handle = thread::spawn(move || {
        let mut net = Session::accept(&listener, &source_id, st).unwrap();
        net.server_negotiate(vec!["chunk-v1".into()]).unwrap();
        source.serve_cut(&mut net).unwrap();
        assert!(
            source.may_mutate(src_session),
            "source owns before transfer"
        );
        let set = source
            .serve_authority_transfer(&mut net, src_session, cut_num)
            .unwrap();
        assert!(
            !source.may_mutate(src_session),
            "source must drop to read-only after relinquishment (AUTH-005)"
        );
        (source, set)
    });

    // Destination: import the cut, then accept the authority transfer.
    let mut net = Session::connect(&addr, &dest_id, dt).unwrap();
    net.client_negotiate(vec!["chunk-v1".into()]).unwrap();
    destination
        .import_cut(&mut net, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .expect("import completes");

    let mirror = Core::mirror_session_id(&sess_str);
    assert!(
        !destination.may_mutate(mirror),
        "imported mirror starts read-only"
    );
    let dest_set = destination
        .request_authority_transfer(&mut net, mirror, EDITOR_ID)
        .unwrap();

    let (source, src_set) = src_handle.join().unwrap();

    // Both sides agree on the receipt set (AUTH-003: durable matched receipts).
    assert_eq!(src_set, dest_set, "both peers hold the same receipt set");
    assert_eq!(dest_set.new_epoch, 1, "epoch advanced 0 -> 1 (monotonic)");

    // Destination is now the authoritative single writer (AUTH-001).
    assert!(
        destination.may_mutate(mirror),
        "destination owns authority after transfer"
    );
    let dstate = destination.authority_state(mirror).unwrap();
    assert_eq!(dstate.owner_device, "local");
    assert_eq!(dstate.epoch.0, 1);
    assert!(!dstate.ambiguous);

    // Source stays read-only even after the connection is gone (AUTH-002: network
    // loss never grants authority back).
    drop(net);
    assert!(
        !source.may_mutate(src_session),
        "source stays read-only off-net (AUTH-002)"
    );
}

#[test]
fn interrupted_commit_leaves_destination_ambiguous_then_recovers() {
    // The destination accepts and opens its pending epoch, then the source dies
    // before sending AuthorityCommit: the dest is left ambiguous, writes blocked
    // (§19.3.6 / AUTH-004), until manual recovery opens a fresh epoch (§21.3).
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, src_session, cut_num) = seal_editor_source(src_dir.path());
    let mut destination = dest_core(dst_dir.path());
    let sess_str = src_session.to_string();

    let (source_id, dest_id, st, dt, listener, addr) = paired_loopback();

    // Source: serve the cut, send the Proposal, read the Accept, then "crash"
    // (drop the connection) before committing relinquishment.
    let src_handle = thread::spawn(move || {
        let mut net = Session::accept(&listener, &source_id, st).unwrap();
        net.server_negotiate(vec!["chunk-v1".into()]).unwrap();
        source.serve_cut(&mut net).unwrap();
        let _pending = source
            .propose_authority_transfer(&mut net, src_session, cut_num)
            .unwrap();
        // Crash before commit: authority never moves, source still owns its epoch.
        drop(net);
        assert!(
            source.may_mutate(src_session),
            "source keeps authority after a crash before commit (AUTH-002)"
        );
    });

    let mut net = Session::connect(&addr, &dest_id, dt).unwrap();
    net.client_negotiate(vec!["chunk-v1".into()]).unwrap();
    destination
        .import_cut(&mut net, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .expect("import completes");
    let mirror = Core::mirror_session_id(&sess_str);

    // The destination accepts, opens the pending (ambiguous) epoch, then blocks on
    // the never-arriving AuthorityCommit — the connection closes → Err.
    let res = destination.request_authority_transfer(&mut net, mirror, EDITOR_ID);
    assert!(res.is_err(), "interrupted transfer must not succeed");

    let _ = src_handle.join();

    // The pending epoch the destination opened is ambiguous: writes are blocked.
    let state = destination.authority_state(mirror).unwrap();
    assert!(state.ambiguous, "interrupted commit leaves ambiguous state");
    assert!(
        !destination.may_mutate(mirror),
        "ambiguous session blocks writes (AUTH-004)"
    );
    assert!(destination.guard_mutation(mirror).is_err());

    // Manual recovery opens a fresh, clean epoch above the ambiguous one (§21.3).
    let new_epoch = destination.recover_authority(mirror).unwrap();
    assert_eq!(
        new_epoch.0, 2,
        "recovery opens epoch above the ambiguous one"
    );
    assert!(
        destination.may_mutate(mirror),
        "after recovery the session is writable again"
    );
    let recovered = destination.authority_state(mirror).unwrap();
    assert!(!recovered.ambiguous);
    assert_eq!(recovered.owner_device, "local");
}

#[test]
fn ambiguous_state_survives_reopen_and_blocks_writes() {
    // AUTH-004 is durable: an ambiguous epoch written before a crash is still
    // ambiguous after the core is reopened and recovery runs.
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, src_session, cut_num) = seal_editor_source(src_dir.path());
    let mut destination = dest_core(dst_dir.path());
    let sess_str = src_session.to_string();

    let (source_id, dest_id, st, dt, listener, addr) = paired_loopback();
    let src_handle = thread::spawn(move || {
        let mut net = Session::accept(&listener, &source_id, st).unwrap();
        net.server_negotiate(vec!["chunk-v1".into()]).unwrap();
        source.serve_cut(&mut net).unwrap();
        let _ = source.propose_authority_transfer(&mut net, src_session, cut_num);
        drop(net);
    });

    let mut net = Session::connect(&addr, &dest_id, dt).unwrap();
    net.client_negotiate(vec!["chunk-v1".into()]).unwrap();
    destination
        .import_cut(&mut net, &sess_str, cut_num)
        .unwrap();
    let mirror = Core::mirror_session_id(&sess_str);
    let _ = destination.request_authority_transfer(&mut net, mirror, EDITOR_ID);
    let _ = src_handle.join();
    assert!(destination.authority_state(mirror).unwrap().ambiguous);

    // Reopen the destination core: recovery re-detects the ambiguous session.
    drop(destination);
    let reopened = Core::open(dst_dir.path()).unwrap();
    assert!(
        reopened.authority_state(mirror).unwrap().ambiguous,
        "ambiguity is durable across reopen (AUTH-004)"
    );
    assert!(!reopened.may_mutate(mirror), "still blocked after reopen");
    assert!(reopened
        .recovery_report()
        .ambiguous_sessions
        .iter()
        .any(|s| *s == mirror.to_string()));
}
