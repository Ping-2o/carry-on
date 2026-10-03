//! Session + cut invariants: stable session identity (CORE-001), immutable
//! sealed cut (CORE-002), object version/schema/length/digest (CORE-003).

use carryon_core::db::Db;

#[test]
fn sealed_cut_update_is_rejected() {
    // CORE-002: the trigger must abort any UPDATE of a sealed cut row.
    let db = Db::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO cuts (session_id, cut_number, epoch, adapter_schema_version, \
             manifest_digest, sealed) VALUES ('s', 0, 0, 1, 'deadbeef', 1)",
            [],
        )
        .unwrap();

    let err = db
        .conn()
        .execute(
            "UPDATE cuts SET manifest_digest='tampered' WHERE session_id='s' AND cut_number=0",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("sealed cut is immutable"),
        "sealed cut must be immutable, got: {err}"
    );

    // An unsealed cut can still be updated (it is only proposed).
    db.conn()
        .execute(
            "INSERT INTO cuts (session_id, cut_number, epoch, adapter_schema_version, \
             manifest_digest, sealed) VALUES ('s', 1, 0, 1, 'cafe', 0)",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "UPDATE cuts SET sealed=1 WHERE session_id='s' AND cut_number=1",
            [],
        )
        .unwrap();
}
