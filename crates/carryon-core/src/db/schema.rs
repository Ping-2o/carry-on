//! SQLite schema (spec §19.2), schema version 1. All tables are created in one
//! transactional migration (CORE-007).
//!
//! Phase-2-only columns (device keys, source signatures) are present so the
//! schema is stable across phases, but are unused in Phase 1.

/// The full schema v1 as one SQL batch. A SQLite trigger enforces cut
/// immutability (CORE-002): any UPDATE of a `sealed=1` cut row raises.
pub const SCHEMA_V1: &str = r#"
CREATE TABLE schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_utc TEXT NOT NULL,
    checksum    TEXT NOT NULL
);

CREATE TABLE devices (
    device_id        TEXT PRIMARY KEY,
    display_name     TEXT,
    device_class     TEXT,
    trust_generation INTEGER NOT NULL DEFAULT 0,
    created_utc      TEXT
);

CREATE TABLE device_keys (
    device_id TEXT PRIMARY KEY REFERENCES devices(device_id),
    key_ref   TEXT NOT NULL
);

CREATE TABLE adapters (
    adapter_id        TEXT PRIMARY KEY,
    adapter_version   TEXT NOT NULL,
    integration_level TEXT NOT NULL,
    manifest_json     TEXT NOT NULL,
    manifest_digest   TEXT NOT NULL,
    registered_utc    TEXT NOT NULL
);

CREATE TABLE adapter_consents (
    consent_id  TEXT PRIMARY KEY,
    adapter_id  TEXT NOT NULL REFERENCES adapters(adapter_id),
    scope_json  TEXT NOT NULL,
    granted_utc TEXT NOT NULL,
    revoked_utc TEXT
);

CREATE TABLE sessions (
    session_id      TEXT PRIMARY KEY,
    adapter_id      TEXT NOT NULL,
    adapter_version TEXT NOT NULL,
    schema_version  INTEGER NOT NULL,
    title           TEXT,
    creation_device TEXT,
    created_utc     TEXT,
    authority_epoch INTEGER NOT NULL,
    latest_cut      INTEGER,
    privacy         TEXT NOT NULL,
    manifest_root   TEXT,
    state           TEXT NOT NULL,
    generation      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE cuts (
    session_id             TEXT NOT NULL,
    cut_number             INTEGER NOT NULL,
    epoch                  INTEGER NOT NULL,
    adapter_schema_version INTEGER NOT NULL,
    derived_validity_json  TEXT,
    source_signature       BLOB,
    created_utc            TEXT,
    manifest_digest        TEXT NOT NULL,
    sealed                 INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, cut_number)
);

-- CORE-002: a sealed cut is immutable. Reject any UPDATE of a sealed row.
CREATE TRIGGER cuts_sealed_immutable
BEFORE UPDATE ON cuts
WHEN OLD.sealed = 1
BEGIN
    SELECT RAISE(ABORT, 'OBJECT_Invalid: sealed cut is immutable');
END;

CREATE TABLE cut_objects (
    session_id TEXT NOT NULL,
    cut_number INTEGER NOT NULL,
    ordinal    INTEGER NOT NULL,
    object_id  TEXT NOT NULL,
    generation INTEGER NOT NULL,
    role       TEXT NOT NULL,
    PRIMARY KEY (session_id, cut_number, ordinal)
);

CREATE TABLE objects (
    object_id    TEXT NOT NULL,
    generation   INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    schema_id    TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    wire_hash    TEXT,
    logical_size INTEGER NOT NULL,
    wire_size    INTEGER,
    recipe_id    TEXT,
    portable     INTEGER NOT NULL,
    sensitivity  TEXT NOT NULL,
    retention    TEXT NOT NULL,
    parents_json TEXT NOT NULL,
    created_utc  TEXT NOT NULL,
    PRIMARY KEY (object_id, generation)
);

CREATE TABLE object_locations (
    content_hash TEXT PRIMARY KEY,
    rel_path     TEXT NOT NULL,
    present      INTEGER NOT NULL,
    verified_utc TEXT
);

CREATE TABLE recipes (
    recipe_id    TEXT PRIMARY KEY,
    impl_version TEXT NOT NULL,
    params_json  TEXT NOT NULL
);

CREATE TABLE actions (
    action_id      TEXT PRIMARY KEY,
    session_id     TEXT NOT NULL,
    cut_number     INTEGER,
    class          TEXT NOT NULL,
    params_json    TEXT NOT NULL,
    mutates        INTEGER NOT NULL,
    result_json    TEXT,
    oracle_outcome TEXT,
    created_utc    TEXT NOT NULL
);

CREATE TABLE transfers (
    transfer_id   TEXT PRIMARY KEY,
    object_id     TEXT NOT NULL,
    generation    INTEGER NOT NULL,
    content_hash  TEXT NOT NULL,
    wire_hash     TEXT,
    total_length  INTEGER NOT NULL,
    chunk_size    INTEGER NOT NULL,
    state         TEXT NOT NULL,
    staging_path  TEXT,
    created_utc   TEXT NOT NULL,
    updated_utc   TEXT NOT NULL
);

CREATE TABLE transfer_chunks (
    transfer_id  TEXT NOT NULL,
    chunk_index  INTEGER NOT NULL,
    chunk_digest TEXT NOT NULL,
    length       INTEGER NOT NULL,
    acked        INTEGER NOT NULL,
    PRIMARY KEY (transfer_id, chunk_index)
);

CREATE TABLE authority_epochs (
    session_id   TEXT NOT NULL,
    epoch        INTEGER NOT NULL,
    owner_device TEXT NOT NULL,
    mode         TEXT NOT NULL,
    opened_utc   TEXT NOT NULL,
    closed_utc   TEXT,
    ambiguous    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, epoch)
);

CREATE TABLE idempotency_records (
    idempotency_key TEXT PRIMARY KEY,
    request_digest  TEXT NOT NULL,
    result_json     TEXT,
    terminal_state  TEXT,
    created_utc     TEXT NOT NULL
);

CREATE TABLE journal_entries (
    seq            INTEGER PRIMARY KEY AUTOINCREMENT,
    event_uuid     TEXT NOT NULL,
    monotonic_ns   TEXT NOT NULL,
    utc            TEXT NOT NULL,
    device         TEXT,
    process        TEXT,
    session_id     TEXT,
    cut_number     INTEGER,
    transfer_id    TEXT,
    action_id      TEXT,
    event_type     TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    prev_hash      TEXT NOT NULL,
    result_code    TEXT NOT NULL,
    metadata_json  TEXT
);

CREATE TABLE evidence_bundles (
    evidence_id      TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL,
    created_utc      TEXT NOT NULL,
    manifest_json    TEXT NOT NULL,
    bundle_path      TEXT NOT NULL,
    verifier_version TEXT NOT NULL
);

CREATE TABLE retention_holds (
    hold_id     TEXT PRIMARY KEY,
    object_id   TEXT,
    generation  INTEGER,
    session_id  TEXT,
    reason      TEXT NOT NULL,
    created_utc TEXT NOT NULL
);
"#;

/// Schema v2 (Phase 3): adds graceful-suspend resume state to transfers. Additive
/// ALTER only (CORE-007 one-transaction migration); existing v1 DBs upgrade clean.
/// `acked_offset` is the verified-staged byte offset recorded at a cooperative
/// suspend; the transfers `state` column gains a new `'suspended'` value (no schema
/// change needed, the column is free-form text). A `'suspended'` transfer keeps its
/// staging `.tmp` file and is resumable (§11.2/§19.3), unlike a crash-interrupted
/// `'staging'`/`'verifying'` transfer which recovery discards.
pub const SCHEMA_V2: &str = r#"
ALTER TABLE transfers ADD COLUMN acked_offset INTEGER NOT NULL DEFAULT 0;
"#;

/// Schema v3 (Phase 4): durable authority-transfer receipts (§21.2 / AUTH-003).
/// A commit MAY proceed only once the required receipt set is durable: the
/// destination's acceptance receipt and the source's relinquishment receipt are
/// each written here (one row per `(proposal_id, role)`) before the epoch moves.
/// The row therefore survives a crash, so recovery can tell a half-finished
/// transfer (receipts present, epoch not yet moved) from an ambiguous one and act
/// per §19.3.6. Additive CREATE only (CORE-007 one-transaction migration).
pub const SCHEMA_V3: &str = r#"
CREATE TABLE authority_receipts (
    proposal_id  TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    cut_number   INTEGER NOT NULL,
    new_epoch    INTEGER NOT NULL,
    role         TEXT NOT NULL,   -- 'source' | 'destination'
    receipt      TEXT NOT NULL,
    created_utc  TEXT NOT NULL,
    PRIMARY KEY (proposal_id, role)
);
"#;

/// Current schema version.
pub const SCHEMA_VERSION: i64 = 3;
