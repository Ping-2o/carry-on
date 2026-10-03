//! Evidence bundle (§23.3, EVD-003/005/006): completeness, hash integrity, and
//! verification that never executes bundle content.

use carryon_adapter_graph::GraphAdapter;
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

#[test]
fn bundle_is_complete_and_verifies() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
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
    core.execute_action(
        cut,
        ActionRequest {
            class: "graph.shortest_path".into(),
            params: serde_json::json!({ "start": 0, "end": 4 }),
        },
    )
    .unwrap();

    let bundle = core.export_evidence(session).unwrap();
    // Completeness: journal, actions, metrics, disclosure, per-section hashes.
    assert!(!bundle.journal.is_empty());
    assert_eq!(bundle.actions.len(), 1);
    assert!(bundle.disclosure.contains("does NOT prove"));
    assert_eq!(bundle.section_hashes.len(), 4);
    // Integrity: the bundle verifies against its own section hashes.
    assert!(bundle.verify().ok);
}

#[test]
fn verify_from_disk_never_executes_content() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
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
    core.create_cut(session).unwrap();
    let bundle = core.export_evidence(session).unwrap();

    let path = dir.path().join("evidence.json");
    std::fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();
    // verify_evidence only parses + hash-checks (EVD-005).
    assert!(core.verify_evidence(&path).unwrap().ok);

    // Tampering with a section without updating its hash is detected.
    let mut tampered = bundle.clone();
    tampered.metrics = serde_json::json!({ "sealed_cuts": 9999 });
    let bad_path = dir.path().join("tampered.json");
    std::fs::write(&bad_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert!(!core.verify_evidence(&bad_path).unwrap().ok);
}
