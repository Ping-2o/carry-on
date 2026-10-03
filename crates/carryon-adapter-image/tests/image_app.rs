//! Image application test (§26.5.2): render_region runs with the dual-render
//! oracle agreeing; the heightfield is published into the store.

use carryon_adapter_image::{HeightField, ImageAdapter};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

#[test]
fn render_region_runs_and_oracle_agrees() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core
        .register_adapter(Box::new(ImageAdapter::sample()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "terrain".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();

    let req = ActionRequest {
        class: "image.render_region".into(),
        params: serde_json::json!({ "x": 1, "y": 1, "w": 3, "h": 3 }),
    };
    let result = core.execute_action(cut, req).unwrap();
    assert!(result.oracle.agreed, "direct and tiled renders must match");
    assert_eq!(result.output.get("bytes").and_then(|v| v.as_u64()), Some(9));
}

#[test]
fn larger_field_still_renders() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let field = HeightField {
        width: 16,
        height: 16,
        cells: (0..256).map(|i| i as u8).collect(),
    };
    let info = core
        .register_adapter(Box::new(ImageAdapter::new(field)))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "terrain".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    let req = ActionRequest {
        class: "image.render_region".into(),
        params: serde_json::json!({ "x": 0, "y": 0, "w": 16, "h": 16 }),
    };
    let result = core.execute_action(cut, req).unwrap();
    assert!(result.oracle.agreed);
    assert_eq!(
        result.output.get("bytes").and_then(|v| v.as_u64()),
        Some(256)
    );
}
