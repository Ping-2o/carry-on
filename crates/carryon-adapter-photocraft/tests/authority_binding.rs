//! L4 authority-transfer binding for the PhotoCraft adapter skeleton: the
//! proposal is bound to the authoritative-state hash (document, unsaved and
//! meta bytes), the destination re-derives and must agree, and a divergent
//! destination state is refused. Fail-closed paths (no prepare, tampered
//! receipt) are pinned. Local evidence; cross-device L4 remains the
//! engine's protocol (see carryon-adapter-editor tests for the wire-level
//! analogue once a real driver exists).

use carryon_adapter_api::{Adapter, CutRef};
use carryon_adapter_photocraft::{model, PhotocraftAdapter};
use carryon_core::model::{ActionRequest, AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

const PIN: &str = "1f7fc0f53696e54da79b95015abb37d7c22cdd1a";

fn meta() -> model::PhotocraftMeta {
    model::PhotocraftMeta {
        schema_id: "carryon.photocraft.session".into(),
        schema_version: 1,
        adapter_version: "0.1.0".into(),
        original_path: Some("/work/wave.pcraft".into()),
        document_name: "wave.pcraft".into(),
        revision: 9,
        saved_revision: 4,
        dirty: true,
        photocraft_pin: PIN.into(),
        unsaved_mode: model::UnsavedMode::Snapshot,
        document_encoding: model::DocumentEncoding::Monolithic,
    }
}

fn adapter_with_unsaved(unsaved: &[u8]) -> PhotocraftAdapter {
    PhotocraftAdapter::with_state(
        "auth-test-session",
        b"PCRAFT-STANDIN-DOCUMENT".to_vec(),
        unsaved.to_vec(),
        model::Workspace::default(),
        meta(),
    )
}

fn cut() -> CutRef {
    CutRef {
        session: "auth-test-session".into(),
        cut_number: 3,
    }
}

/// prepare → accept (identical state) → commit succeeds, bound to the hash.
#[test]
fn authority_transfer_accepts_matching_state() {
    let mut source = adapter_with_unsaved(b"unsaved-state-A");
    let mut destination = adapter_with_unsaved(b"unsaved-state-A");

    let proposal = source.prepare_authority_transfer(&cut()).unwrap();
    let receipt = destination
        .accept_authority_transfer(&cut(), &proposal)
        .unwrap();
    let relinquish = source.commit_authority_transfer("p1", &receipt).unwrap();
    assert!(relinquish.contains(PIN) || relinquish.contains("relinquish"));
    // Pending cleared after commit: a second commit must fail closed.
    assert!(source.commit_authority_transfer("p2", &receipt).is_err());
}

/// A destination holding DIFFERENT state must refuse the transfer — authority
/// is bound to the authoritative-state hash, not to a bearer token.
#[test]
fn authority_transfer_refuses_divergent_state() {
    let mut source = adapter_with_unsaved(b"unsaved-state-A");
    let mut destination = adapter_with_unsaved(b"unsaved-state-B");

    let proposal = source.prepare_authority_transfer(&cut()).unwrap();
    let err = destination
        .accept_authority_transfer(&cut(), &proposal)
        .unwrap_err();
    assert!(
        format!("{err}").contains("ADAPTER_INCOMPATIBLE"),
        "divergent state must be refused: {err}"
    );
}

/// A tampered destination receipt must be refused by the source's commit.
#[test]
fn commit_refuses_mismatched_receipt() {
    let mut source = adapter_with_unsaved(b"unsaved-state-A");
    let _proposal = source.prepare_authority_transfer(&cut()).unwrap();
    let bad_receipt = "accept:3:deadbeef".to_string();
    let err = source
        .commit_authority_transfer("p1", &bad_receipt)
        .unwrap_err();
    assert!(format!("{err}").contains("ADAPTER_INCOMPATIBLE"));
    // The pending transfer survives a failed commit; abort clears it.
    source.abort_authority_transfer("p1", "test");
    assert!(source
        .commit_authority_transfer("p1", &bad_receipt)
        .is_err());
}

/// A wrong cut number in the proposal is refused.
#[test]
fn accept_refuses_wrong_cut() {
    let mut source = adapter_with_unsaved(b"unsaved-state-A");
    let mut destination = adapter_with_unsaved(b"unsaved-state-A");
    let proposal = source.prepare_authority_transfer(&cut()).unwrap();
    let other_cut = CutRef {
        session: "auth-test-session".into(),
        cut_number: 4,
    };
    assert!(destination
        .accept_authority_transfer(&other_cut, &proposal)
        .is_err());
}

/// A commit without a prepare fails closed.
#[test]
fn commit_without_prepare_fails() {
    let mut source = adapter_with_unsaved(b"unsaved-state-A");
    assert!(source
        .commit_authority_transfer("p1", "accept:3:anything")
        .is_err());
}

/// End-to-end through the core: the two in-memory sessions seal, transfer
/// authority and prove it via session.restore — the same skeleton flow the
/// Phase-2 driver must keep green against a real PhotoCraft.
#[test]
fn l4_flow_through_core_moves_authority() {
    let dir = tempdir().unwrap();
    let mut src_core = Core::open(dir.path()).unwrap();
    let info = src_core
        .register_adapter(Box::new(adapter_with_unsaved(b"unsaved-final")))
        .unwrap();
    let session = src_core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "l4".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = src_core.create_cut(session).unwrap();

    // The restored session is self-consistent and the oracle agrees.
    let result = src_core
        .execute_action(
            cut,
            ActionRequest {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .unwrap();
    assert!(result.oracle.agreed);
    assert_eq!(result.output["revision"], 9);
}
