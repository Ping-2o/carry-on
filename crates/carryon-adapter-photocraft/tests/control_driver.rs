//! Driver-wiring tests for the Phase-2 ControlChannelDriver seam: attached
//! state refresh on begin_snapshot, live-restore fail-closed paths, and the
//! EVD-006 token redaction guarantees. Protocol-level client tests live in
//! `src/control.rs` (mock server); the live-app experiment is
//! `examples/l3_live.rs` + `docs/artcraft-bridge/evidence/l3-live/`.

use carryon_adapter_api::{Adapter, SnapshotToken};
use carryon_adapter_photocraft::{PhotocraftAdapter, StubDriver};

/// begin_snapshot with the stub attached fails closed (no live state to pull).
#[test]
fn begin_snapshot_with_stub_driver_fails_closed() {
    let a = PhotocraftAdapter::session_v1().with_driver(Box::new(StubDriver));
    let mut a = a;
    let err = a.begin_snapshot("s", 1).unwrap_err();
    assert!(
        format!("{err}").contains("document_bytes"),
        "must name the failing driver call: {err}"
    );
}

/// session.restore with the stub attached fails closed (live restore refused,
/// not faked).
#[test]
fn session_restore_with_stub_driver_fails_closed() {
    let mut a = PhotocraftAdapter::session_v1().with_driver(Box::new(StubDriver));
    let cut = carryon_adapter_api::CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let err = a
        .execute_action(
            &cut,
            &carryon_adapter_api::ActionRequestWire {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("unsupported"),
        "stub live restore must fail closed: {err}"
    );
}

/// Detached (in-memory) begin_snapshot and restore keep working unchanged —
/// the Phase-1 contract is not disturbed by the driver seam.
#[test]
fn detached_mode_unchanged() {
    let mut a = PhotocraftAdapter::session_v1();
    let token = a.begin_snapshot("s", 1).unwrap();
    let manifest = a.describe_snapshot(&token).unwrap();
    assert_eq!(manifest.objects.len(), 4);
    let cut = carryon_adapter_api::CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let r = a
        .execute_action(
            &cut,
            &carryon_adapter_api::ActionRequestWire {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .unwrap();
    assert!(r.oracle.agreed);
    assert_eq!(r.output["live_restore"], serde_json::Value::Null);
}

/// A screenshot token never enters Debug output of the config (EVD-006).
#[test]
fn control_config_debug_redacts_token() {
    use carryon_adapter_photocraft::control::ControlChannelConfig;
    let cfg = ControlChannelConfig {
        addr: "127.0.0.1:7878".into(),
        token: carryon_adapter_photocraft::Secret::new("very-secret-token"),
        base_document: "/tmp/base.pcraft".into(),
        write_root: "/tmp/w".into(),
        read_root: "/tmp/r".into(),
    };
    let dbg = format!("{cfg:?}");
    assert!(!dbg.contains("very-secret-token"), "EVD-006: {dbg}");
    assert!(dbg.contains("<redacted"));
}

/// `SnapshotToken`/restore round-trip sanity for the object server: the four
/// objects serve through read_object in ranges (live-mode state refresh is
/// driver-tested above; this pins the serving contract).
#[test]
fn read_object_serves_all_four_objects() {
    let a = PhotocraftAdapter::session_v1();
    for id in [
        "photocraft.document.v1",
        "photocraft.unsaved_edits.v1",
        "photocraft.meta.v1",
        "photocraft.workspace.v1",
    ] {
        let bytes = a
            .read_object(&SnapshotToken("t".into()), id, 0, u64::MAX)
            .unwrap();
        assert!(!bytes.is_empty(), "{id} must serve non-empty bytes");
    }
}
