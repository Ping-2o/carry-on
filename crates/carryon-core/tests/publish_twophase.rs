//! Two-phase publish (spec §18.6/§19.1): digest verify, quarantine on mismatch,
//! idempotent dedup, incomplete-never-visible. CORE-004, OBJECT_DigestMismatch,
//! NET-006.

mod common;

use carryon_core::db::Db;
use carryon_core::ids::Digest;
use carryon_core::journal::Journal;
use carryon_core::store::publish::publish_object;
use carryon_core::store::Store;
use common::{object_of, Bytes, LyingBytes};
use tempfile::tempdir;

fn harness(dir: &std::path::Path) -> (Db, Store, Journal) {
    let db = Db::open(&dir.join("metadata.sqlite3")).unwrap();
    let store = Store::open(dir).unwrap();
    std::fs::create_dir_all(dir.join("journals")).unwrap();
    let (journal, _events, _trunc) = Journal::open(&dir.join("journals/events.log")).unwrap();
    (db, store, journal)
}

#[test]
fn publishes_and_is_present() {
    let dir = tempdir().unwrap();
    let (mut db, store, mut journal) = harness(dir.path());

    let bytes = b"hello carry-on".to_vec();
    let obj = object_of("o.v1", &bytes);
    let out = publish_object(
        &mut db,
        &store,
        &mut journal,
        &obj,
        &Bytes(bytes.clone()),
        None,
    )
    .unwrap();

    assert!(!out.deduplicated);
    assert!(store.exists(&obj.content_hash));
    // The objects row is present only after publish.
    let n: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM objects WHERE object_id='o.v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn digest_mismatch_quarantines_and_commits_nothing() {
    let dir = tempdir().unwrap();
    let (mut db, store, mut journal) = harness(dir.path());

    // Declare the hash of the TRUTH but serve different bytes.
    let truth = b"the real bytes".to_vec();
    let obj = object_of("o.v1", &truth);
    let lying = LyingBytes {
        served: b"tampered bytes!".to_vec(),
    };

    let err = publish_object(&mut db, &store, &mut journal, &obj, &lying, None).unwrap_err();
    assert_eq!(err.family(), "OBJECT");

    // Nothing committed; nothing present.
    assert!(!store.exists(&obj.content_hash));
    let n: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM objects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "no object row on digest mismatch (CORE-004)");

    // Quarantine holds the suspect bytes (failure evidence preserved).
    let q = std::fs::read_dir(dir.path().join("quarantine"))
        .unwrap()
        .count();
    assert_eq!(q, 1);
}

#[test]
fn republish_is_idempotent() {
    let dir = tempdir().unwrap();
    let (mut db, store, mut journal) = harness(dir.path());

    let bytes = b"same bytes".to_vec();
    let obj = object_of("o.v1", &bytes);
    publish_object(
        &mut db,
        &store,
        &mut journal,
        &obj,
        &Bytes(bytes.clone()),
        None,
    )
    .unwrap();
    let again = publish_object(
        &mut db,
        &store,
        &mut journal,
        &obj,
        &Bytes(bytes.clone()),
        None,
    )
    .unwrap();
    assert!(
        again.deduplicated,
        "second publish of same digest is a no-op (NET-006)"
    );

    let n: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM object_locations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn small_chunks_still_verify() {
    let dir = tempdir().unwrap();
    let (mut db, store, mut journal) = harness(dir.path());

    let bytes: Vec<u8> = (0..4096u32).map(|i| (i % 256) as u8).collect();
    let obj = object_of("o.v1", &bytes);
    // 7-byte chunks exercise the chunk loop + per-chunk digest path.
    let out = publish_object(
        &mut db,
        &store,
        &mut journal,
        &obj,
        &Bytes(bytes.clone()),
        Some(7),
    )
    .unwrap();
    assert_eq!(out.digest, Digest::of(&bytes));
    assert!(store.exists(&obj.content_hash));
}
