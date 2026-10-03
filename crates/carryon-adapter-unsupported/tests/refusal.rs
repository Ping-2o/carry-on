//! Honest-refusal test (§26.5.6, §30): the unsupported adapter refuses a
//! structured snapshot/action cleanly instead of faking readiness.

use carryon_adapter_unsupported::UnsupportedAdapter;
use carryon_core::model::{AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

#[test]
fn create_cut_is_refused_honestly() {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core.register_adapter(Box::new(UnsupportedAdapter)).unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "unsupported".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();

    // Sealing a cut must fail honestly: the adapter exposes no typed state.
    let err = core.create_cut(session).unwrap_err();
    assert_eq!(err.family(), "ADAPTER");
    assert!(
        err.to_string().contains("no typed state") || err.to_string().contains("Incompatible"),
        "refusal must be explicit, got: {err}"
    );

    // The adapter still registers (ADP-001) — refusal is at snapshot time, not
    // registration. It advertises L0 (activation only).
    assert_eq!(
        core.get_adapter("org.carryon.unsupported")
            .unwrap()
            .integration_level,
        carryon_adapter_api::IntegrationLevel::L0
    );
}
