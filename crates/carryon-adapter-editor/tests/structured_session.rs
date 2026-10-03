//! L3 structured continuation: a full working session (document + unsaved edits +
//! navigation + meta) is sealed as a multi-object cut, and the read-only
//! `session.restore` action reports the restored logical session with an agreeing
//! oracle. Local evidence (one core); the cross-device carry is proven by the
//! `carryon_l3` shell. Labeled L3 STRUCTURED CONTINUATION, not process migration.

use carryon_adapter_editor::EditorAdapter;
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

#[test]
fn structured_session_seals_multiobject_and_restores() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core
        .register_adapter(Box::new(EditorAdapter::session_v1()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "l3".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();

    // Seal a cut: only the three Authoritative objects (document, unsaved, meta) seal
    // into the cut; navigation is Ephemeral/optional and is NOT sealed.
    let cut = core.create_cut(session).unwrap();
    let ids = core.cut_authoritative_objects(cut);
    assert!(
        ids.iter().any(|s| s == "editor.document.v1"),
        "document sealed: {ids:?}"
    );
    assert!(
        ids.iter().any(|s| s == "editor.unsaved_edits.v1"),
        "unsaved sealed: {ids:?}"
    );
    assert!(
        ids.iter().any(|s| s == "editor.meta.v1"),
        "meta sealed: {ids:?}"
    );
    assert!(
        !ids.iter().any(|s| s == "editor.navigation.v1"),
        "navigation is optional, must NOT seal into the cut: {ids:?}"
    );

    // session.restore: read-only, oracle agrees, reports the recognizable navigation.
    let result = core
        .execute_action(
            cut,
            ActionRequest {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .unwrap();
    assert!(result.oracle.agreed, "restore oracle must agree");
    let nav = &result.output["navigation"];
    assert_eq!(nav["cursor"], 42);
    assert_eq!(nav["selection_anchor"], 20);
    assert_eq!(nav["selection_head"], 25);
    assert_eq!(nav["scroll_y"], 128);
    assert_eq!(nav["active_tab"], "draft.md");
}

#[test]
fn edit_continues_after_restore() {
    // The destination (here, one core) can continue editing: document.edit mutates
    // the unsaved buffer and bumps generation, oracle agrees.
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core
        .register_adapter(Box::new(EditorAdapter::session_v1()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "l3".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    let result = core
        .execute_action(
            cut,
            ActionRequest {
                class: "document.edit".into(),
                params: serde_json::json!({ "text": "continued on destination [edit]" }),
            },
        )
        .unwrap();
    assert!(result.oracle.agreed);
    assert_eq!(result.output["generation"], 2);
}
