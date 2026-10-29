//! Integration tests for the Phase-1 Kiwix adapter skeleton.
//!
//! These prove what is IMPLEMENTED (object model, chunked serving, dependency
//! plans, L4 proposal protocol over the reading position). Everything that
//! would need libzim is asserted to fail CLOSED with an UNSUPPORTED reason —
//! a negative test that must keep passing until Phase 2 replaces the stub.

use carryon_adapter_api::*;
use carryon_adapter_kiwix::*;
use sha2::{Digest, Sha256};

const CHUNK: u64 = 1024;

/// Chunk-read an object through `read_object` and reassemble it, proving the
/// offset/length path the core's chunked transfer uses.
fn read_all(adapter: &KiwixAdapter, token: &SnapshotToken, object_id: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offset = 0u64;
    loop {
        let part = adapter
            .read_object(token, object_id, offset, CHUNK)
            .expect("chunk read");
        if part.is_empty() {
            return out;
        }
        out.extend_from_slice(&part);
        offset += part.len() as u64;
    }
}

fn manifest_of(adapter: &mut KiwixAdapter) -> (SnapshotToken, ObjectManifest) {
    let gen = adapter.current_generation("s").expect("gen");
    let token = adapter.begin_snapshot("s", gen).expect("begin snapshot");
    let manifest = adapter.describe_snapshot(&token).expect("manifest");
    (token, manifest)
}

fn entry<'a>(m: &'a ObjectManifest, id: &str) -> &'a ObjectEntry {
    m.objects
        .iter()
        .find(|o| o.object_id == id)
        .unwrap_or_else(|| panic!("object {id} in manifest"))
}

#[test]
fn manifest_kinds_respect_the_core_entry_cap() {
    // No bulk attached: the bulk object is NOT declared at all (nothing to
    // serve — an empty-digest entry would fail the core's validate_entry).
    let mut adapter = KiwixAdapter::sample();
    let (_, m) = manifest_of(&mut adapter);
    assert!(m.objects.iter().all(|o| o.object_id != "kiwix.zimfile.v1"));
    assert_eq!(
        entry(&m, "kiwix.session.v1").kind,
        ObjectKindWire::Authoritative
    );
    // The immediate-tier object is Authoritative (sealed): first paint must
    // ride the cut without the bulk.
    assert_eq!(
        entry(&m, "kiwix.article.v1").kind,
        ObjectKindWire::Authoritative
    );
    // …and records the lineage: parents = the ZIM bulk object.
    assert_eq!(
        entry(&m, "kiwix.article.v1").parents,
        vec![ObjectVersionWire {
            object_id: "kiwix.zimfile.v1".into(),
            generation: m.generation,
        }]
    );
    for id in [
        "kiwix.bookmarks.v1",
        "kiwix.history.v1",
        "kiwix.settings.v1",
        "kiwix.meta.v1",
    ] {
        assert_eq!(entry(&m, id).kind, ObjectKindWire::Authoritative, "{id}");
    }

    // Bulk attached (under the core's 16 MiB entry cap): declared as Cache
    // in Progressive mode.
    let mut with_bulk = KiwixAdapter::sample().with_zim_bytes(vec![7u8; 4096]);
    let (_, m) = manifest_of(&mut with_bulk);
    assert_eq!(entry(&m, "kiwix.zimfile.v1").kind, ObjectKindWire::Cache);

    // Bulk above the cap: NOT declared (L1 object outside the cut instead).
    let big = vec![9u8; (16 * 1024 * 1024 + 1) as usize];
    let mut over = KiwixAdapter::sample().with_zim_bytes(big);
    let (_, m) = manifest_of(&mut over);
    assert!(m.objects.iter().all(|o| o.object_id != "kiwix.zimfile.v1"));
}

#[test]
fn full_upfront_strategy_seals_the_declared_bulk_into_the_cut() {
    // Strategy A: the declared bulk kind flips to Authoritative — the
    // engine's mechanical "everything up front" (it seals and transfers).
    let mut a = KiwixAdapter::sample()
        .with_zim_bytes(vec![7u8; 4096])
        .with_strategy(Strategy::FullUpFront);
    let (_, m) = manifest_of(&mut a);
    assert_eq!(
        entry(&m, "kiwix.zimfile.v1").kind,
        ObjectKindWire::Authoritative
    );
}

#[test]
fn restore_plan_puts_bulk_and_userdata_in_optional() {
    // Bulk attached (under the core entry cap) so the plan tiering is visible.
    let adapter = KiwixAdapter::sample().with_zim_bytes(vec![7u8; 4096]);
    let cut = CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let plan = adapter
        .resolve_action(
            &cut,
            &ActionRequestWire {
                class: "reader.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .expect("plan");
    let ids =
        |v: &Vec<ObjectVersionWire>| v.iter().map(|o| o.object_id.clone()).collect::<Vec<_>>();
    // Prerequisite: session + article (immediate tier) + meta — KBs, not GBs.
    assert_eq!(ids(&plan.prerequisites).len(), 3);
    assert!(ids(&plan.prerequisites).contains(&"kiwix.session.v1".into()));
    assert!(ids(&plan.prerequisites).contains(&"kiwix.article.v1".into()));
    // Optional: bookmarks, history, settings, AND the 80 GB ZIM bulk.
    assert!(ids(&plan.optional).contains(&"kiwix.bookmarks.v1".into()));
    assert!(ids(&plan.optional).contains(&"kiwix.history.v1".into()));
    assert!(ids(&plan.optional).contains(&"kiwix.settings.v1".into()));
    assert!(ids(&plan.optional).contains(&"kiwix.zimfile.v1".into()));
}

#[test]
fn article_serves_chunked_and_reassembles() {
    let mut adapter = KiwixAdapter::sample();
    let (token, m) = manifest_of(&mut adapter);
    let declared = entry(&m, "kiwix.article.v1");
    assert!(declared.logical_size > 0);

    let reassembled = read_all(&adapter, &token, "kiwix.article.v1");
    assert_eq!(reassembled.len() as u64, declared.logical_size);
    assert!(reassembled.windows(5).any(|w| w == b"<html"));
    // Declared digest must equal the reassembled bytes' digest.
    let mut h = Sha256::new();
    h.update(&reassembled);
    let mut hex = String::new();
    for b in h.finalize() {
        hex.push_str(&format!("{b:02x}"));
    }
    assert_eq!(declared.content_hash, hex);
}

#[test]
fn zim_bulk_serves_chunked_when_attached_and_fails_closed_when_not() {
    // Attached: chunked bulk serving works end to end (Phase-1 stand-in for
    // the libzim backend).
    let bulk: Vec<u8> = (0..(3 * CHUNK + 37) as u32)
        .map(|i| (i % 251) as u8)
        .collect();
    let mut adapter = KiwixAdapter::sample().with_zim_bytes(bulk.clone());
    let (token, m) = manifest_of(&mut adapter);
    let declared = entry(&m, "kiwix.zimfile.v1");
    assert_eq!(declared.logical_size, bulk.len() as u64);

    let reassembled = read_all(&adapter, &token, "kiwix.zimfile.v1");
    assert_eq!(reassembled, bulk);

    // Not attached: the bulk is not declared at all, and reads fail CLOSED
    // with an UNSUPPORTED reason — never fabricated bytes, never a silent
    // empty success.
    let mut bare = KiwixAdapter::sample();
    let (token, m) = manifest_of(&mut bare);
    assert!(m.objects.iter().all(|o| o.object_id != "kiwix.zimfile.v1"));
    let err = bare
        .read_object(&token, "kiwix.zimfile.v1", 0, CHUNK)
        .expect_err("must fail closed without libzim backend");
    match err {
        AdapterError::Incompatible(msg) => {
            assert!(msg.contains("UNSUPPORTED"), "reason must be labeled: {msg}")
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
}

#[test]
fn navigate_moves_the_position_and_invalidates_generation() {
    let mut adapter = KiwixAdapter::sample();
    let gen0 = adapter.generation();
    let cut = CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let result = adapter
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "reader.navigate".into(),
                params: serde_json::json!({"path": "A/Closure_(computer_programming)", "title": "Closure", "scroll_y": 7700}),
            },
        )
        .expect("navigate");
    assert!(result.oracle.agreed);
    assert_eq!(adapter.generation(), gen0 + 1);
    assert_eq!(
        adapter.state().position.article_path,
        "A/Closure_(computer_programming)"
    );
    assert_eq!(adapter.state().position.scroll_y, 7700);

    // Staleness: a snapshot begun at the old generation is refused.
    let err = adapter
        .begin_snapshot("s", gen0)
        .expect_err("stale generation must be refused");
    match err {
        AdapterError::StaleGeneration { expected, actual } => {
            assert_eq!((expected, actual), (gen0, gen0 + 1));
        }
        other => panic!("expected StaleGeneration, got {other:?}"),
    }
}

#[test]
fn authority_transfer_binds_to_position_state_not_the_book() {
    let cut = CutRef {
        session: "s".into(),
        cut_number: 1,
    };

    // Source prepares; destination accepts (same state => same auth hash).
    let mut source = KiwixAdapter::sample().with_zim_bytes(vec![7u8; 4096]);
    let mut dest = KiwixAdapter::sample().with_zim_bytes(vec![9u8; 4096]);
    let proposal = source.prepare_authority_transfer(&cut).expect("prepare");
    let receipt = dest
        .accept_authority_transfer(&cut, &proposal)
        .expect("accept");
    let relinquish = source
        .commit_authority_transfer("p1", &receipt)
        .expect("commit");
    assert!(relinquish.starts_with("relinquish:"));

    // The ZIM bulk differs between source and destination but authority is
    // over the POSITION state: transfer still succeeds (hash excludes the
    // ZIM — proven above by both sides succeeding with different bulk bytes).

    // Tampered destination state => hash mismatch => refused.
    let mut tampered = KiwixAdapter::sample().with_zim_bytes(vec![9u8; 4096]);
    tampered
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "reader.navigate".into(),
                params: serde_json::json!({"path": "A/Other", "scroll_y": 1}),
            },
        )
        .expect("navigate on tampered copy");
    let err = tampered
        .accept_authority_transfer(&cut, &proposal)
        .expect_err("tampered state must be refused");
    assert!(matches!(err, AdapterError::Incompatible(_)));

    // Commit with a foreign receipt => refused, pending preserved.
    let mut source2 = KiwixAdapter::sample();
    let _ = source2.prepare_authority_transfer(&cut).expect("prepare");
    let err = source2
        .commit_authority_transfer("p2", "accept:1:deadbeef")
        .expect_err("foreign receipt must be refused");
    assert!(matches!(err, AdapterError::Incompatible(_)));
    // Abort clears pending.
    source2.abort_authority_transfer("p2", "test");
    let err = source2
        .commit_authority_transfer("p2", "whatever")
        .expect_err("no pending transfer after abort");
    assert!(matches!(err, AdapterError::Incompatible(_)));
}

#[test]
fn materialize_validates_ranges_and_fails_closed_without_bulk() {
    let cut = CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let mut with_bulk = KiwixAdapter::sample().with_zim_bytes(vec![1u8; 4096]);
    let ok = with_bulk
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "library.materialize".into(),
                params: serde_json::json!({"offset": 0, "length": 1024}),
            },
        )
        .expect("materialize ack");
    assert_eq!(ok.output["accepted"], serde_json::json!(true));

    // Out of range: refused (fail closed).
    let err = with_bulk
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "library.materialize".into(),
                params: serde_json::json!({"offset": 4090, "length": 100}),
            },
        )
        .expect_err("over-range must be refused");
    assert!(matches!(err, AdapterError::OutOfRange(_)));

    // No bulk attached: UNSUPPORTED, fail closed.
    let mut bare = KiwixAdapter::sample();
    let err = bare
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "library.materialize".into(),
                params: serde_json::json!({"offset": 0, "length": 10}),
            },
        )
        .expect_err("no libzim backend: fail closed");
    match err {
        AdapterError::Incompatible(msg) => {
            assert!(msg.contains("UNSUPPORTED"), "labeled: {msg}")
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
}

#[test]
fn unknown_actions_and_objects_fail_closed() {
    let mut adapter = KiwixAdapter::sample();
    let cut = CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let err = adapter
        .resolve_action(
            &cut,
            &ActionRequestWire {
                class: "book.rebind".into(),
                params: serde_json::json!({}),
            },
        )
        .expect_err("unknown action");
    assert!(matches!(err, AdapterError::ActionUnsupported(_)));

    let gen = adapter.current_generation("s").unwrap();
    let token = adapter.begin_snapshot("s", gen).unwrap();
    let err = adapter
        .read_object(&token, "kiwix.nonexistent.v1", 0, 1)
        .expect_err("unknown object");
    assert!(matches!(err, AdapterError::UnknownObject(_)));
}
