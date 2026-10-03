//! Crash recovery (§19.3, CORE-008): an interrupted publish never leaves a
//! visible valid object, interrupted transfers are reported, and a present=1 row
//! without a verified file is hidden. Nothing is relabeled success.

use carryon_core::db::Db;
use carryon_core::Core;
use tempfile::tempdir;

/// Simulate a crash mid-publish: a transfers row in 'staging' plus a stray
/// staging/*.tmp, with no committed object. Then open a Core (which runs
/// recovery) and assert cleanup.
#[test]
fn interrupted_transfer_is_discarded_and_reported() {
    let dir = tempdir().unwrap();
    let data = dir.path();

    // Lay out a data dir with a half-written transfer.
    {
        let db = Db::open(&data.join("metadata.sqlite3")).unwrap();
        db.conn()
            .execute(
                "INSERT INTO transfers (transfer_id, object_id, generation, content_hash, \
                 total_length, chunk_size, state, staging_path, created_utc, updated_utc) \
                 VALUES ('t1','o.v1',1,'00ff','10','1024','staging','x','u','u')",
                [],
            )
            .unwrap();
    }
    std::fs::create_dir_all(data.join("staging")).unwrap();
    std::fs::write(data.join("staging/t1.tmp"), b"partial").unwrap();

    // Open the engine: recovery runs.
    let core = Core::open(data).unwrap();
    let report = core.recovery_report();
    assert!(
        report.interrupted_transfers.iter().any(|t| t == "t1"),
        "interrupted transfer must be reported"
    );
    assert!(!report.is_clean());

    // The stray staging file is gone; the transfer is marked failed; no object
    // became visible.
    assert!(!data.join("staging/t1.tmp").exists());
    let db = Db::open(&data.join("metadata.sqlite3")).unwrap();
    let state: String = db
        .conn()
        .query_row(
            "SELECT state FROM transfers WHERE transfer_id='t1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "failed");
    let objs: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM objects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        objs, 0,
        "no object may be visible after an interrupted publish (CORE-004)"
    );
}

/// A present=1 location whose file is missing must be hidden on recovery.
#[test]
fn present_without_file_is_hidden() {
    let dir = tempdir().unwrap();
    let data = dir.path();
    {
        let db = Db::open(&data.join("metadata.sqlite3")).unwrap();
        db.conn()
            .execute(
                "INSERT INTO object_locations (content_hash, rel_path, present, verified_utc) \
                 VALUES ('abcd','objects/sha256/ab/cd/abcd',1,'u')",
                [],
            )
            .unwrap();
    }
    let core = Core::open(data).unwrap();
    assert!(!core.recovery_report().hidden_incomplete_objects.is_empty());

    let db = Db::open(&data.join("metadata.sqlite3")).unwrap();
    let present: i64 = db
        .conn()
        .query_row(
            "SELECT present FROM object_locations WHERE content_hash='abcd'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(present, 0, "a location without a file must be hidden");
}

/// A session left mid-preparation by a crash becomes Inconclusive, never success.
#[test]
fn interrupted_session_is_inconclusive() {
    let dir = tempdir().unwrap();
    let data = dir.path();
    {
        let db = Db::open(&data.join("metadata.sqlite3")).unwrap();
        db.conn()
            .execute(
                "INSERT INTO sessions (session_id, adapter_id, adapter_version, schema_version, \
                 authority_epoch, privacy, state, generation) \
                 VALUES ('s','a','1',1,0,'public','Preparing',0)",
                [],
            )
            .unwrap();
    }
    let _core = Core::open(data).unwrap();
    let db = Db::open(&data.join("metadata.sqlite3")).unwrap();
    let state: String = db
        .conn()
        .query_row("SELECT state FROM sessions WHERE session_id='s'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        state, "Inconclusive",
        "interrupted session must not be relabeled success"
    );
}
