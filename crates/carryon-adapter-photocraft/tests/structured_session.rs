//! L3 structured continuation for the PhotoCraft adapter skeleton: the four
//! objects seal/publish with the right classification, bytes serve, and the
//! read-only `session.restore` reports the recognizable workspace position
//! with an agreeing oracle. Local evidence (one core). The Phase-2 driver that
//! would make this real against PhotoCraft is UNSUPPORTED (ADAPTER-DESIGN §5).

use carryon_adapter_photocraft::{model, PhotocraftAdapter, StubDriver};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::{tempdir, TempDir};

/// The TempDir is returned so it outlives the Core (the editor tests keep it
/// alive the same way; dropping it early deletes the store under the core).
fn core_with_session() -> (Core, TempDir, carryon_core::ids::SessionId) {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core
        .register_adapter(Box::new(PhotocraftAdapter::session_v1()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "photocraft-l3".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    (core, dir, session)
}

#[test]
fn photocraft_session_seals_authoritative_and_publishes_workspace() {
    let (mut core, _dir, session) = core_with_session();

    // Seal a cut: only the three Authoritative objects (document, unsaved,
    // meta) seal; the workspace object is Ephemeral/optional and is NOT sealed.
    let cut = core.create_cut(session).unwrap();
    let ids = core.cut_authoritative_objects(cut);
    assert!(
        ids.iter().any(|s| s == "photocraft.document.v1"),
        "document sealed: {ids:?}"
    );
    assert!(
        ids.iter().any(|s| s == "photocraft.unsaved_edits.v1"),
        "unsaved sealed: {ids:?}"
    );
    assert!(
        ids.iter().any(|s| s == "photocraft.meta.v1"),
        "meta sealed: {ids:?}"
    );
    assert!(
        !ids.iter().any(|s| s == "photocraft.workspace.v1"),
        "workspace is optional, must NOT seal into the cut: {ids:?}"
    );

    // session.restore: read-only, oracle agrees, reports the recognizable
    // workspace position + document identity (revision/dirty from meta).
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
    assert_eq!(result.output["revision"], 7);
    assert_eq!(result.output["dirty"], true);
    assert_eq!(result.output["selected_layers"][0]["name"], "Curves 1");
    assert_eq!(result.output["viewport"]["zoom"], 0.5);
    assert_eq!(result.output["active_tool"], "brush");
}

#[test]
fn read_object_serves_document_bytes_in_ranges() {
    let a = PhotocraftAdapter::session_v1();
    use carryon_adapter_api::Adapter;
    let token = carryon_adapter_api::SnapshotToken("snap-1".into());
    let all = a
        .read_object(&token, "photocraft.document.v1", 0, u64::MAX)
        .unwrap();
    assert!(!all.is_empty());
    // offset+length clamps like the reference adapter.
    let mid = a
        .read_object(&token, "photocraft.document.v1", 2, 4)
        .unwrap();
    assert_eq!(mid.as_slice(), &all[2..6]);
    assert!(a.read_object(&token, "photocraft.nope", 0, 1).is_err());
}

#[test]
fn document_edit_fails_closed_without_a_driver() {
    let (mut core, _dir, session) = core_with_session();
    let cut = core.create_cut(session).unwrap();
    let err = core
        .execute_action(
            cut,
            ActionRequest {
                class: "document.edit".into(),
                // The exact journal shape PhotoCraft's actions/batch consume
                // (RESEARCH §1.3): [["paint.stroke", {...}]].
                params: serde_json::json!({
                    "steps": [["paint.stroke", {"points": [[10, 10, 1.0]], "size": 20}]]
                }),
            },
        )
        .unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("action_unsupported"),
        "edit must fail closed without a driver: {msg}"
    );
    assert!(
        msg.contains("phase 2"),
        "the refusal must name the missing Phase-2 driver: {msg}"
    );
}

#[test]
fn document_edit_with_stub_driver_still_fails_closed() {
    let mut a = PhotocraftAdapter::session_v1().with_driver(Box::new(StubDriver));
    use carryon_adapter_api::Adapter;
    let cut = carryon_adapter_api::CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let res = a.execute_action(
        &cut,
        &carryon_adapter_api::ActionRequestWire {
            class: "document.edit".into(),
            params: serde_json::json!({"steps": [["edit.undo", null]]}),
        },
    );
    let err = res.unwrap_err();
    assert!(
        format!("{err}").contains("ACTION_UNSUPPORTED"),
        "stub driver must fail closed, not fake success: {err}"
    );
}

#[test]
fn document_edit_rejects_malformed_steps() {
    let (mut core, _dir, session) = core_with_session();
    let cut = core.create_cut(session).unwrap();
    for bad in [
        serde_json::json!({}),                            // no steps
        serde_json::json!({"steps": "nope"}),             // not an array
        serde_json::json!({"steps": [["id", "params"]]}), // params not object
        serde_json::json!({"steps": [["id"]]}),           // not a pair
    ] {
        let err = core
            .execute_action(
                cut,
                ActionRequest {
                    class: "document.edit".into(),
                    params: bad,
                },
            )
            .unwrap_err();
        assert!(
            format!("{err}").contains("steps"),
            "malformed params must fail closed naming 'steps': {err}"
        );
    }
}

#[test]
fn meta_carries_session_side_identity() {
    // The .pcraft file has no revision field (RESEARCH §1.4); the meta object
    // is what carries it. Pin the meta shape the adapter serves.
    let a = PhotocraftAdapter::session_v1();
    use carryon_adapter_api::Adapter;
    let token = carryon_adapter_api::SnapshotToken("snap-1".into());
    let bytes = a
        .read_object(&token, "photocraft.meta.v1", 0, u64::MAX)
        .unwrap();
    let meta: model::PhotocraftMeta = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(meta.revision, 7);
    assert_eq!(meta.saved_revision, 3);
    assert!(meta.dirty);
    assert_eq!(
        meta.photocraft_pin,
        "1f7fc0f53696e54da79b95015abb37d7c22cdd1a"
    );
    assert_eq!(meta.unsaved_mode, model::UnsavedMode::Snapshot);
}
