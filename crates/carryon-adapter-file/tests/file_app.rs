//! File application test (§26.5.3): L1 continuation by content-hash identity.

use carryon_adapter_file::FileAdapter;
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

#[test]
fn opens_document_by_content_identity() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core
        .register_adapter(Box::new(FileAdapter::sample()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "doc".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();

    // The authoritative document is present after the cut.
    let avail = core.list_available_actions(cut, &["document.open".into()]);
    assert!(avail[0].ready);

    let req = ActionRequest {
        class: "document.open".into(),
        params: serde_json::json!({ "uri": "file:///tmp/notes.txt" }),
    };
    let result = core.execute_action(cut, req).unwrap();
    // L1 open is activation-only: no oracle claim.
    assert!(!result.oracle.checked);
    assert_eq!(
        result.output.get("opened").and_then(|v| v.as_str()),
        Some("file:///tmp/notes.txt")
    );
}

#[test]
fn continues_a_real_file_from_disk() {
    let dir = tempdir().unwrap();
    let doc = dir.path().join("report.md");
    std::fs::write(&doc, b"# real file on disk\ncarry-on L1\n").unwrap();

    let adapter = FileAdapter::from_path(&doc).unwrap();
    assert!(adapter.uri().starts_with("file://"));
    assert!(adapter.uri().ends_with("report.md"));

    let mut core = Core::open(dir.path()).unwrap();
    let info = core.register_adapter(Box::new(adapter)).unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "disk doc".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    // The file's exact bytes publish under their content hash and the open action
    // is ready — continuation by content identity, not by in-memory state.
    let cut = core.create_cut(session).unwrap();
    assert!(core.list_available_actions(cut, &["document.open".into()])[0].ready);
}
