//! Schema migrations (CORE-007): v1+v2+v3 apply transactionally; re-open idempotent.

use carryon_core::db::Db;
use tempfile::tempdir;

#[test]
fn migrates_and_reopens_idempotently() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("metadata.sqlite3");

    let db = Db::open(&path).unwrap();
    let v: i64 = db
        .conn()
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(v, 3);
    drop(db);

    // Re-open: no error, still version 3 (idempotent).
    let db2 = Db::open(&path).unwrap();
    let v2: i64 = db2
        .conn()
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(v2, 3);
    // v2 column present.
    let has_col: i64 = db2
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('transfers') WHERE name='acked_offset'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_col, 1, "v2 acked_offset column missing");
    // v3 authority_receipts table present.
    let has_table: i64 = db2
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='authority_receipts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_table, 1, "v3 authority_receipts table missing");
}

#[test]
fn in_memory_has_all_core_tables() {
    let db = Db::open_in_memory().unwrap();
    for t in [
        "sessions",
        "cuts",
        "cut_objects",
        "objects",
        "object_locations",
        "transfers",
        "transfer_chunks",
        "authority_epochs",
        "authority_receipts",
        "idempotency_records",
        "journal_entries",
        "evidence_bundles",
        "retention_holds",
        "adapters",
        "adapter_consents",
    ] {
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [t],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "missing table {t}");
    }
}
