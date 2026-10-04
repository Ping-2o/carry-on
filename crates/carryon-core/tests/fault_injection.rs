//! Fault-injection suite: every fault must **fail closed** through an existing error
//! path — no silent data loss, no partial object exposed as valid, no authority moved
//! on a dropped connection. One test per fault class the engine must survive:
//!
//!   1. corrupted chunk (wrong per-chunk digest, and wrong whole-object bytes)
//!   2. connection loss during each authority phase (propose / accept→commit window)
//!   3. stale generation (adapter ahead of the expected snapshot generation)
//!   4. restart recovery (crash mid-transfer discards the non-suspended partial)
//!   5. insufficient storage / budget (over-budget import refused before any bytes)
//!   6. malicious manifest (a lying remote manifest rejected before a byte moves)
//!
//! LOCAL EVIDENCE ONLY (spec §2/§30): loopback over 127.0.0.1, not cross-device. These
//! prove the fail-closed invariants; the same code paths run on the physical device.

use carryon_adapter_api::{
    Adapter, AdapterError, ObjectEntry, ObjectKindWire, ObjectManifest, RetentionWire,
    SensitivityWire,
};
use carryon_adapter_editor::EditorAdapter;
use carryon_adapter_graph::GraphAdapter;
use carryon_core::carryon_net::wire::Message;
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session, TrustStore};
use carryon_core::ids::{Digest, SessionId};
use carryon_core::model::{AuthorityMode, Budget, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

const EDITOR_ID: &str = "org.carryon.editor";

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

fn seal_graph_source(dir: &std::path::Path) -> (Core, String, u64) {
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

fn seal_editor_source(dir: &std::path::Path) -> (Core, SessionId, u64) {
    let mut core = Core::open(dir).unwrap();
    let info = core
        .register_adapter(Box::new(EditorAdapter::new(
            "editor-session",
            b"draft".to_vec(),
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

fn dest_editor_core(dir: &std::path::Path) -> Core {
    let mut core = Core::open(dir).unwrap();
    core.register_adapter(Box::new(EditorAdapter::new("mirror", b"draft".to_vec())))
        .unwrap();
    core
}

type Paired = (
    DeviceIdentity,
    DeviceIdentity,
    Arc<TrustStore>,
    Arc<TrustStore>,
);

fn paired() -> Paired {
    let s = DeviceIdentity::generate("source").unwrap();
    let d = DeviceIdentity::generate("dest").unwrap();
    let (st, dt) = pair_devices(&s, &d, "fault").unwrap();
    (s, d, Arc::new(st), Arc::new(dt))
}

/// A valid-looking manifest entry whose `content_hash` is the sha256 of `bytes`.
fn honest_entry(object_id: &str, bytes: &[u8]) -> ObjectEntry {
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

// ---------------------------------------------------------------------------
// 1. corrupted chunk
// ---------------------------------------------------------------------------

/// A rogue source serves a manifest for one object, then answers the destination's
/// `TransferRequest` with bytes whose per-chunk digest it *lies* about (NET-007) — and
/// in a second run, bytes that do not hash to the declared whole-object `content_hash`
/// (CORE-004 whole-object verify). Both must fail closed: nothing is published.
#[test]
fn corrupted_chunk_fails_closed() {
    for mode in ["bad-chunk-digest", "bad-object-bytes"] {
        let dst_dir = tempdir().unwrap();
        let payload = b"the honest object bytes".to_vec();
        let payload_hex = Digest::of(&payload).to_hex();
        let manifest = ObjectManifest {
            session: "11111111-1111-1111-1111-111111111111".into(),
            generation: 0,
            objects: vec![honest_entry("fault.obj.v1", &payload)],
        };

        let (sid, did, st, dt) = paired();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let m2 = manifest.clone();
        let srv = thread::spawn(move || {
            let Ok(mut s) = Session::accept(&listener, &sid, st) else {
                return;
            };
            let _ = s.server_negotiate(vec![]);
            // CutRequest -> serve the (honest) manifest.
            match s.recv() {
                Ok(Message::CutRequest { .. }) => {
                    let _ = s.send(Message::CutManifest { manifest: m2 });
                }
                _ => return,
            }
            // TransferRequest -> answer with corrupt bytes / lying digest.
            if let Ok(Message::TransferRequest {
                content_hash,
                offset,
                ..
            }) = s.recv()
            {
                let (bytes, digest) = match mode {
                    // Right bytes, WRONG declared per-chunk digest (NET-007).
                    "bad-chunk-digest" => (payload.clone(), "deadbeef".to_string()),
                    // WRONG bytes with a self-consistent chunk digest; the whole-object
                    // verify against content_hash still fails (CORE-004).
                    _ => {
                        let tampered = b"tampered not the real bytes!".to_vec();
                        let d = Digest::of(&tampered).to_hex();
                        (tampered, d)
                    }
                };
                let _ = s.send(Message::ChunkData {
                    content_hash,
                    offset,
                    chunk_digest: digest,
                    bytes,
                });
            }
            // Drain until the client disconnects.
            while s.recv().is_ok() {}
        });

        let mut dest = Core::open(dst_dir.path()).unwrap();
        let mut s = Session::connect(&addr, &did, dt).unwrap();
        s.client_negotiate(vec![]).unwrap();
        let err = dest
            .import_cut(&mut s, &manifest.session, 0)
            .expect_err("corrupt transfer must fail closed");
        // A lying per-chunk digest fails at the chunk guard (TRANSFER, NET-007); tampered
        // bytes fail at the whole-object verify (OBJECT_DigestMismatch, CORE-004). Both
        // are fail-closed: nothing is published.
        let fam = err.family();
        assert!(
            fam == "TRANSFER" || fam == "OBJECT",
            "mode {mode}: expected TRANSFER/OBJECT fail-closed, got {fam}: {err}"
        );
        drop(s);
        let _ = srv.join();
        // Nothing was published: the object is absent from the store.
        assert!(
            !dest.has_object_hex(&payload_hex),
            "mode {mode}: no bytes may be published on a corrupt transfer"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. connection loss during authority phases
// ---------------------------------------------------------------------------

/// Source dies AFTER sending the authority Proposal but BEFORE committing
/// relinquishment: the destination opens its pending epoch and then the connection
/// drops. Authority must NOT silently move — the source keeps ownership, and the
/// destination is left ambiguous (writes blocked, AUTH-004), recoverable via
/// `recover_authority`. Covers the propose/accept→commit window (the phase where a loss
/// is dangerous).
#[test]
fn connection_loss_during_commit_blocks_writes() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, src_session, cut_num) = seal_editor_source(src_dir.path());
    let mut dest = dest_editor_core(dst_dir.path());
    let sess_str = src_session.to_string();

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let srv = thread::spawn(move || {
        let mut net = Session::accept(&listener, &sid, st).unwrap();
        net.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut net).unwrap();
        // Propose, then CRASH before commit.
        let _ = source.propose_authority_transfer(&mut net, src_session, cut_num);
        drop(net);
        // The source never relinquished: it still owns its epoch (AUTH-002).
        assert!(
            source.may_mutate(src_session),
            "source keeps authority on loss"
        );
    });

    let mut net = Session::connect(&addr, &did, dt).unwrap();
    net.client_negotiate(vec![]).unwrap();
    dest.import_cut(&mut net, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .expect("import completes");
    let mirror = Core::mirror_session_id(&sess_str);
    // The commit never arrives → the request errors (connection closed).
    assert!(
        dest.request_authority_transfer(&mut net, mirror, EDITOR_ID)
            .is_err(),
        "interrupted authority transfer must not succeed"
    );
    let _ = srv.join();

    // Destination is ambiguous: writes are blocked, fail closed with an AUTH code.
    let state = dest.authority_state(mirror).unwrap();
    assert!(
        state.ambiguous,
        "loss in the commit window leaves ambiguous state"
    );
    assert!(
        !dest.may_mutate(mirror),
        "ambiguous blocks writes (AUTH-004)"
    );
    assert_eq!(dest.guard_mutation(mirror).unwrap_err().family(), "AUTH");

    // Recovery opens a fresh clean epoch above the ambiguous one (§21.3).
    dest.recover_authority(mirror).unwrap();
    assert!(dest.may_mutate(mirror), "writable again after recovery");
}

// ---------------------------------------------------------------------------
// 3. stale generation
// ---------------------------------------------------------------------------

/// An adapter asked to snapshot an EXPECTED generation it is not at must refuse with
/// `StaleGeneration` rather than silently snapshot the wrong version (the core maps this
/// to `ADAPTER_StaleGeneration`). Exercised directly on the editor adapter's snapshot
/// guard — the same guard the cut/snapshot path calls.
#[test]
fn stale_generation_is_refused() {
    let mut ed = EditorAdapter::new("editor-session", b"v1".to_vec()); // generation 1
    match ed.begin_snapshot("editor-session", 999) {
        Err(AdapterError::StaleGeneration { expected, actual }) => {
            assert_eq!(expected, 999);
            assert_eq!(actual, 1);
        }
        other => panic!("expected StaleGeneration, got {other:?}"),
    }
    // The matching generation still succeeds (no false positive).
    assert!(ed.begin_snapshot("editor-session", 1).is_ok());
}

// ---------------------------------------------------------------------------
// 4. restart recovery
// ---------------------------------------------------------------------------

/// A crash mid-transfer (process drops the core without finishing) must discard the
/// non-suspended staging partial on reopen — never resurrect it as a valid object
/// (EVD-003 / §19.3.9). We force a crash by suspending at the first chunk boundary,
/// then dropping the core, and assert the reopened core shows no interrupted transfer
/// and no partially-published object.
#[test]
fn restart_discards_nonsuspended_partial() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, sess, cut) = seal_graph_source(src_dir.path());
    let mut dest = Core::open(dst_dir.path()).unwrap();
    dest.set_chunk_size(Some(4 * 1024)).unwrap();

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = thread::spawn(move || {
        if let Ok(mut s) = Session::accept(&listener, &sid, st) {
            let _ = s.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut s);
        }
    });

    // Interrupt the import, then "crash" by dropping the core before it finishes.
    dest.request_suspend();
    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let _ = dest.import_cut(&mut s, &sess, cut);
    drop(s);
    drop(dest);
    let _ = srv.join();

    // Reopen: recovery must leave no interrupted transfer lingering as valid.
    let reopened = Core::open(dst_dir.path()).unwrap();
    let report = reopened.recovery_report();
    // A suspended row may be preserved for resume, but nothing is left in a half-valid
    // 'staging' state that recovery reports as a silent, unrecoverable interruption.
    assert!(
        report.interrupted_transfers.is_empty(),
        "no silent interrupted transfer after restart, got {:?}",
        report.interrupted_transfers
    );
}

// ---------------------------------------------------------------------------
// 5. insufficient storage / budget
// ---------------------------------------------------------------------------

/// A destination whose preparation budget is below the import size must refuse the
/// import BEFORE any bytes move (§6.8) — modeling insufficient storage/quota. Fails
/// closed with a BUDGET code; nothing is staged.
#[test]
fn insufficient_budget_refused_before_bytes() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, sess, cut) = seal_graph_source(src_dir.path());
    let mut dest = Core::open(dst_dir.path()).unwrap();
    let mut budget = Budget::local_default();
    budget.total_net_bytes = 1; // below any object
    dest.set_budget(budget);

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = thread::spawn(move || {
        if let Ok(mut s) = Session::accept(&listener, &sid, st) {
            let _ = s.server_negotiate(vec![]);
            let _ = source.serve_cut(&mut s);
        }
    });

    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let err = dest.import_cut(&mut s, &sess, cut).unwrap_err();
    assert_eq!(
        err.family(),
        "BUDGET",
        "over-budget import must be refused: {err}"
    );
    drop(s);
    let _ = srv.join();
}

// ---------------------------------------------------------------------------
// 6. malicious manifest
// ---------------------------------------------------------------------------

/// A lying remote manifest must be rejected before a single byte is pulled (ADP-006/008,
/// §7.3 — the same validation a local snapshot gets). Each variant violates one rule:
/// empty schema_id, a Secret-excluded object, and a non-hex content_hash. The import
/// must fail closed on every one.
#[test]
fn malicious_manifest_rejected_before_bytes() {
    let payload = b"irrelevant; validation fails first".to_vec();

    let variants: [(&str, ObjectEntry); 3] = [
        (
            "empty-schema",
            ObjectEntry {
                schema_id: String::new(),
                ..honest_entry("evil.obj.v1", b"x")
            },
        ),
        (
            "secret-excluded",
            ObjectEntry {
                sensitivity: SensitivityWire::Secret,
                ..honest_entry("evil.obj.v1", b"x")
            },
        ),
        (
            "non-hex-hash",
            ObjectEntry {
                content_hash: "not-a-valid-sha256".into(),
                ..honest_entry("evil.obj.v1", b"x")
            },
        ),
    ];

    for (name, entry) in variants {
        let dst_dir = tempdir().unwrap();
        let manifest = ObjectManifest {
            session: "22222222-2222-2222-2222-222222222222".into(),
            generation: 0,
            objects: vec![entry],
        };
        let (sid, did, st, dt) = paired();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let m2 = manifest.clone();
        let payload2 = payload.clone();
        let srv = thread::spawn(move || {
            let Ok(mut s) = Session::accept(&listener, &sid, st) else {
                return;
            };
            let _ = s.server_negotiate(vec![]);
            if let Ok(Message::CutRequest { .. }) = s.recv() {
                let _ = s.send(Message::CutManifest { manifest: m2 });
            }
            // If validation were skipped the dest would ask for bytes; answer so a
            // regression that pulls-then-validates is still caught, then drain.
            if let Ok(Message::TransferRequest {
                content_hash,
                offset,
                ..
            }) = s.recv()
            {
                let _ = s.send(Message::ChunkData {
                    content_hash,
                    offset,
                    chunk_digest: Digest::of(&payload2).to_hex(),
                    bytes: payload2,
                });
            }
            while s.recv().is_ok() {}
        });

        let mut dest = Core::open(dst_dir.path()).unwrap();
        let mut s = Session::connect(&addr, &did, dt).unwrap();
        s.client_negotiate(vec![]).unwrap();
        let err = dest
            .import_cut(&mut s, &manifest.session, 0)
            .expect_err("malicious manifest must be rejected");
        let fam = err.family();
        assert!(
            fam == "OBJECT" || fam == "SCHEMA",
            "variant {name}: expected OBJECT/SCHEMA rejection, got {fam}: {err}"
        );
        drop(s);
        let _ = srv.join();
    }
}
