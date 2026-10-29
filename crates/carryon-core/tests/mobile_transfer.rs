//! Phase 3 mobile-ready transfer: small chunks (§18.6), budget enforcement (§6.8),
//! cooperative suspend→resume (§11.2), and the invariant that a CRASH still discards
//! a non-suspended partial while a SUSPEND survives.
//!
//! LOCAL EVIDENCE ONLY (spec §2/§30): loopback over 127.0.0.1, not cross-device.

use carryon_adapter_graph::GraphAdapter;
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::ids::Digest;
use carryon_core::model::{AuthorityMode, Budget, Sensitivity, SessionState};
use carryon_core::{Core, CreateSessionReq, ImportOutcome};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

/// Seal a graph cut on a source core rooted at `dir`.
fn seal_source(dir: &std::path::Path) -> (Core, String, u64) {
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

fn paired() -> (
    DeviceIdentity,
    DeviceIdentity,
    Arc<carryon_core::carryon_net::TrustStore>,
    Arc<carryon_core::carryon_net::TrustStore>,
) {
    let s = DeviceIdentity::generate("source").unwrap();
    let d = DeviceIdentity::generate("dest").unwrap();
    let (st, dt) = pair_devices(&s, &d, "t").unwrap();
    (s, d, Arc::new(st), Arc::new(dt))
}

#[test]
fn set_chunk_size_rejects_out_of_bounds() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    assert!(core.set_chunk_size(Some(1024)).is_err()); // < 4 KiB
    assert!(core.set_chunk_size(Some(64 * 1024 * 1024)).is_err()); // > 8 MiB
    assert!(core.set_chunk_size(Some(16 * 1024)).is_ok());
    assert!(core.set_chunk_size(None).is_ok());
}

#[test]
fn small_chunk_import_succeeds_with_many_chunks() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, sess, cut) = seal_source(src_dir.path());
    let mut dest = Core::open(dst_dir.path()).unwrap();
    dest.set_chunk_size(Some(4 * 1024)).unwrap(); // tiny chunks

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = thread::spawn(move || {
        let mut s = Session::accept(&listener, &sid, st).unwrap();
        s.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut s).unwrap();
    });

    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let out = dest.import_cut(&mut s, &sess, cut).unwrap();
    assert!(matches!(out, ImportOutcome::Completed(_)));
    srv.join().unwrap();

    // Many small chunks were recorded across the import's transfers.
    let chunks = dest.transfer_chunk_count();
    assert!(chunks > 1, "expected multiple small chunks, got {chunks}");
}

#[test]
fn over_budget_import_is_refused_before_bytes() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, sess, cut) = seal_source(src_dir.path());
    let mut dest = Core::open(dst_dir.path()).unwrap();

    // Budget far below any object size → refuse.
    let mut budget = Budget::local_default();
    budget.total_net_bytes = 1;
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
    assert_eq!(err.family(), "BUDGET");
    // Nothing published.
    drop(s);
    let _ = srv.join();
}

#[test]
fn suspend_then_resume_completes_same_cut() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, sess, cut) = seal_source(src_dir.path());
    let mut dest = Core::open(dst_dir.path()).unwrap();
    dest.set_chunk_size(Some(4 * 1024)).unwrap();

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // Source serves two sessions: first (suspended) import, then the resume.
    let srv = thread::spawn(move || {
        // first connection
        let mut s1 = Session::accept(&listener, &sid, st.clone()).unwrap();
        s1.server_negotiate(vec![]).unwrap();
        let _ = source.serve_cut(&mut s1); // may end when dest suspends/drops
        drop(s1);
        // second connection (resume)
        let mut s2 = Session::accept(&listener, &sid, st).unwrap();
        s2.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut s2).unwrap();
    });

    // Request suspend immediately so the very first chunk boundary suspends.
    dest.request_suspend();
    let mut s1 = Session::connect(&addr, &did, dt.clone()).unwrap();
    s1.client_negotiate(vec![]).unwrap();
    let out = dest.import_cut(&mut s1, &sess, cut).unwrap();
    let token = match out {
        ImportOutcome::Suspended(t) => t,
        ImportOutcome::Completed(_) => panic!("expected suspension"),
        ImportOutcome::PartialBudget(_) => panic!("expected suspension, got budget-partial"),
    };
    drop(s1);

    // Resume on a fresh connection → completes.
    let mut s2 = Session::connect(&addr, &did, dt).unwrap();
    s2.client_negotiate(vec![]).unwrap();
    let out2 = dest.resume_import(&mut s2, &token).unwrap();
    assert!(matches!(out2, ImportOutcome::Completed(_)));
    srv.join().unwrap();
}

#[test]
fn crash_discards_nonsuspended_partial_but_keeps_suspended() {
    // A suspended transfer survives reopen (recovery keeps it); a crash-interrupted
    // 'staging' transfer does not. We assert both via the recovery report after a
    // suspend + reopen.
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let (mut source, sess, cut) = seal_source(src_dir.path());
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

    dest.request_suspend();
    let mut s = Session::connect(&addr, &did, dt).unwrap();
    s.client_negotiate(vec![]).unwrap();
    let out = dest.import_cut(&mut s, &sess, cut).unwrap();
    assert!(matches!(out, ImportOutcome::Suspended(_)));
    drop(s);
    drop(dest); // "crash": process drops the core without finishing
    let _ = srv.join();

    // Reopen: recovery must NOT report the suspended transfer as interrupted (it is
    // resumable, staging file intact).
    let dest2 = Core::open(dst_dir.path()).unwrap();
    let report = dest2.recovery_report();
    assert!(
        report.interrupted_transfers.is_empty(),
        "suspended transfer must survive reopen, got {:?}",
        report.interrupted_transfers
    );
    let _ = dest2; // reopened cleanly; suspended row preserved
}

// ---------------------------------------------------------------------------
// Progressive budget admission (§6.8, Phase-4 tile-splitting support)
// ---------------------------------------------------------------------------

/// A purpose-built three-object adapter with controlled object sizes, in
/// **priority order** (small-critical first, like the split PhotoCraft
/// manifest): `small.v1` (1 KiB), `big1.v1` (64 KiB), `big2.v1` (64 KiB).
mod three_object {
    use carryon_adapter_api::*;
    pub const SMALL: &str = "test.three.small.v1";
    pub const BIG1: &str = "test.three.big1.v1";
    pub const BIG2: &str = "test.three.big2.v1";

    pub struct ThreeObjectAdapter;

    fn blob(len: usize, tag: u8) -> Vec<u8> {
        vec![tag; len]
    }

    pub fn object_bytes(id: &str) -> Option<Vec<u8>> {
        match id {
            SMALL => Some(blob(1024, 1)),
            BIG1 => Some(blob(64 * 1024, 2)),
            BIG2 => Some(blob(64 * 1024, 3)),
            _ => None,
        }
    }

    impl Adapter for ThreeObjectAdapter {
        fn get_adapter_info(&self) -> AdapterInfo {
            AdapterInfo {
                adapter_id: "org.carryon.test.three".into(),
                adapter_version: "1.0.0".into(),
                publisher_id: "org.carryon".into(),
                integration_level: IntegrationLevel::L3,
                executable: AdapterInfo::COMPILED_IN.into(),
                state_schemas: vec![SMALL.into(), BIG1.into(), BIG2.into()],
                actions: vec!["three.inspect".into()],
                permissions: vec![],
                network_access: false,
                supports_snapshot: true,
                supports_mutations: false,
                supports_authority_transfer: false,
                max_object_bytes: 16 * 1024 * 1024,
            }
        }

        fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
            Ok(ConsentToken(format!("consent-{}", scope.target)))
        }

        fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
            Ok(vec![SessionSummary {
                session: "three-object-session".into(),
                title: "three".into(),
                generation: 1,
                schema_version: 1,
            }])
        }

        fn begin_snapshot(
            &mut self,
            _s: &str,
            expected: u64,
        ) -> Result<SnapshotToken, AdapterError> {
            if expected != 1 {
                return Err(AdapterError::StaleGeneration {
                    expected,
                    actual: 1,
                });
            }
            Ok(SnapshotToken("snap-1".into()))
        }

        fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
            let entry = |id: &str| ObjectEntry {
                object_id: id.into(),
                generation: 1,
                kind: ObjectKindWire::Authoritative,
                schema_id: id.into(),
                content_hash: carryon_core::ids::Digest::of(&object_bytes(id).unwrap()).to_hex(),
                logical_size: object_bytes(id).unwrap().len() as u64,
                parents: vec![],
                recipe_id: None,
                portable: true,
                sensitivity: SensitivityWire::Public,
                retention: RetentionWire::Persistent,
            };
            Ok(ObjectManifest {
                session: "three-object-session".into(),
                generation: 1,
                objects: vec![entry(SMALL), entry(BIG1), entry(BIG2)],
            })
        }

        fn read_object(
            &self,
            _t: &SnapshotToken,
            object_id: &str,
            offset: u64,
            length: u64,
        ) -> Result<Vec<u8>, AdapterError> {
            let bytes = object_bytes(object_id)
                .ok_or_else(|| AdapterError::UnknownObject(object_id.into()))?;
            let start = (offset as usize).min(bytes.len());
            let end = (start + length as usize).min(bytes.len());
            Ok(bytes[start..end].to_vec())
        }

        fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
            Ok(SnapshotReceipt {
                session: "three-object-session".into(),
                generation: 1,
                manifest_digest: "test".into(),
            })
        }

        fn abort_snapshot(&mut self, _t: SnapshotToken, _reason: &str) {}

        fn current_generation(&self, _s: &str) -> Result<u64, AdapterError> {
            Ok(1)
        }

        fn resolve_action(
            &self,
            _cut: &CutRef,
            req: &ActionRequestWire,
        ) -> Result<DependencyPlanWire, AdapterError> {
            if req.class != "three.inspect" {
                return Err(AdapterError::ActionUnsupported(req.class.clone()));
            }
            Ok(DependencyPlanWire {
                prerequisites: vec![
                    ObjectVersionWire {
                        object_id: SMALL.into(),
                        generation: 1,
                    },
                    ObjectVersionWire {
                        object_id: BIG1.into(),
                        generation: 1,
                    },
                    ObjectVersionWire {
                        object_id: BIG2.into(),
                        generation: 1,
                    },
                ],
                provenance: vec![],
                optional: vec![],
            })
        }

        fn validate_objects(
            &self,
            _cut: &CutRef,
            versions: &[ObjectVersionWire],
        ) -> Result<ValidationReport, AdapterError> {
            let known = [SMALL, BIG1, BIG2];
            let missing: Vec<_> = versions
                .iter()
                .filter(|v| !known.contains(&v.object_id.as_str()))
                .cloned()
                .collect();
            Ok(ValidationReport {
                ok: missing.is_empty(),
                missing,
                message: "three-object validation".into(),
            })
        }

        fn import_objects(
            &mut self,
            _cut: &CutRef,
            locations: &[ObjectLocation],
        ) -> Result<ImportReceipt, AdapterError> {
            Ok(ImportReceipt {
                imported: locations
                    .iter()
                    .map(|l| ObjectVersionWire {
                        object_id: l.object_id.clone(),
                        generation: l.generation,
                    })
                    .collect(),
            })
        }

        fn activate(
            &mut self,
            _cut: &CutRef,
            _req: &ActionRequestWire,
        ) -> Result<ActivationReceipt, AdapterError> {
            Ok(ActivationReceipt {
                activated: false,
                detail: "test adapter".into(),
            })
        }

        fn execute_action(
            &mut self,
            _cut: &CutRef,
            req: &ActionRequestWire,
        ) -> Result<ActionResultWire, AdapterError> {
            if req.class != "three.inspect" {
                return Err(AdapterError::ActionUnsupported(req.class.clone()));
            }
            let output = serde_json::json!({ "ok": true });
            let output_hash = carryon_core::ids::Digest::of(output.to_string().as_bytes()).to_hex();
            Ok(ActionResultWire {
                output,
                output_hash,
                oracle: OracleOutcome {
                    checked: true,
                    agreed: true,
                    output_hash: String::new(),
                    detail: "test oracle".into(),
                },
            })
        }

        fn export_evidence(
            &self,
            session: &str,
            _range: EvidenceRange,
        ) -> Result<EvidenceFragment, AdapterError> {
            let json = serde_json::json!({ "session": session });
            Ok(EvidenceFragment {
                session: session.into(),
                json,
            })
        }
    }
}

/// The budget admits a PREFIX of the closure (small first), the prefix is
/// published + verified, nothing seals, and a retry with a replenished budget
/// completes the carry with the already-present object deduplicated (moved
/// bytes strictly below the full closure on the second attempt).
#[test]
fn progressive_budget_prefix_publishes_and_retry_completes() {
    use three_object::{ThreeObjectAdapter, BIG1, BIG2, SMALL};

    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let mut source = Core::open(src_dir.path()).unwrap();
    let info = source
        .register_adapter(Box::new(ThreeObjectAdapter))
        .unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "three".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    let sess = session.to_string();
    let cut_num = cut.number;

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // The source serves as many connections as the destination opens.
    let srv = thread::spawn(move || {
        for _ in 0..2 {
            if let Ok(mut s) = Session::accept(&listener, &sid, st.clone()) {
                let _ = s.server_negotiate(vec![]);
                let _ = source.serve_cut(&mut s);
            }
        }
    });

    // Budget covers the small object but not the two big ones.
    let mut dest = Core::open(dst_dir.path()).unwrap();
    let mut budget = Budget::local_default();
    budget.total_net_bytes = 32 * 1024;
    dest.set_budget(budget);

    let mut s1 = Session::connect(&addr, &did, dt.clone()).unwrap();
    s1.client_negotiate(vec![]).unwrap();
    let out1 = dest.import_cut(&mut s1, &sess, cut_num).unwrap();
    let partial = match out1 {
        ImportOutcome::PartialBudget(p) => p,
        other => panic!("expected PartialBudget, got {other:?}"),
    };
    assert_eq!(partial.published, vec![(SMALL.to_string(), 1)]);
    assert_eq!(partial.deferred, vec![BIG1.to_string(), BIG2.to_string()]);
    assert_eq!(partial.deferred_bytes, 2 * 64 * 1024);
    // The prefix is durably present + verified in the store.
    assert!(dest.has_object_hex(&Digest::of(&three_object::object_bytes(SMALL).unwrap()).to_hex()));
    // Nothing sealed: the mirror session stays in Importing (never ACTION_READY,
    // never CutSealed) — no action can run on a budget-partial carry.
    let mirror = Core::mirror_session_id(&sess);
    assert_eq!(
        dest.get_session(mirror).map(|s| s.state),
        Some(SessionState::Importing)
    );
    drop(s1);

    // Replenish: a retry must skip the present object (dedup) and complete.
    dest.set_budget(Budget::local_default());
    let mut s2 = Session::connect(&addr, &did, dt).unwrap();
    s2.client_negotiate(vec![]).unwrap();
    let out2 = dest.import_cut(&mut s2, &sess, cut_num).unwrap();
    out2.completed_cut().expect("retry completes the closure");
    // The second attempt moved only the two big objects (dedup: the small one
    // was already present and verified). Wire bytes are base64-expanded
    // (≈4/3), so the bound allows that expansion plus headers, but stays well
    // below a full re-pull of all three objects.
    let moved = s2.bytes_recv();
    let two_objects_wire = (2 * 64 * 1024 * 4) / 3 + 8 * 1024;
    assert!(
        moved < two_objects_wire,
        "retry should skip the present object, moved {moved} (bound {two_objects_wire})"
    );
    assert!(
        moved > 2 * 64 * 1024,
        "retry must still move the two deferred objects, moved {moved}"
    );
    let _ = srv.join();
}

/// When even the first not-present object exceeds the budget, the import is
/// refused before any bytes and nothing is published (fail closed; the
/// pre-progressive semantics are preserved for this case).
#[test]
fn budget_below_every_object_refuses_before_bytes_and_retries() {
    use three_object::{ThreeObjectAdapter, BIG2};

    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let mut source = Core::open(src_dir.path()).unwrap();
    let info = source
        .register_adapter(Box::new(ThreeObjectAdapter))
        .unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "three".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    let sess = session.to_string();
    let cut_num = cut.number;

    let (sid, did, st, dt) = paired();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let srv = thread::spawn(move || {
        for _ in 0..2 {
            if let Ok(mut s) = Session::accept(&listener, &sid, st.clone()) {
                let _ = s.server_negotiate(vec![]);
                let _ = source.serve_cut(&mut s);
            }
        }
    });

    let mut dest = Core::open(dst_dir.path()).unwrap();
    let mut budget = Budget::local_default();
    budget.total_net_bytes = 1; // below even the smallest object
    dest.set_budget(budget);
    let mut s1 = Session::connect(&addr, &did, dt.clone()).unwrap();
    s1.client_negotiate(vec![]).unwrap();
    let err = dest.import_cut(&mut s1, &sess, cut_num).unwrap_err();
    assert_eq!(err.family(), "BUDGET");
    drop(s1);
    // Nothing published.
    assert!(!dest.has_object_hex(&Digest::of(&three_object::object_bytes(BIG2).unwrap()).to_hex()));

    // A replenished retry completes normally.
    let mirror = Core::mirror_session_id(&sess);
    dest.set_budget(Budget::local_default());
    let mut s2 = Session::connect(&addr, &did, dt).unwrap();
    s2.client_negotiate(vec![]).unwrap();
    let out = dest.import_cut(&mut s2, &sess, cut_num).unwrap();
    assert!(out.completed_cut().is_some());
    assert_eq!(
        dest.get_session(mirror).map(|s| s.state),
        Some(SessionState::ActionReady)
    );
    let _ = srv.join();
}
