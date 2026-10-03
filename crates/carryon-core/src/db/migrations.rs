//! Transactional schema migrations (CORE-007). Phase 1 has a single version.

use super::schema::{SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_VERSION};
use crate::error::{CoreError, InternalCode, Result};
use crate::ids::Digest;
use rusqlite::Connection;

/// Apply all pending migrations inside one transaction. Idempotent: re-opening
/// a migrated DB is a no-op.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn.transaction()?;
    if current < 1 {
        tx.execute_batch(SCHEMA_V1)
            .map_err(|e| CoreError::internal(InternalCode::Io, format!("schema v1: {e}")))?;
        let checksum = Digest::of(SCHEMA_V1.as_bytes()).to_hex();
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_utc, checksum) VALUES (?1, ?2, ?3)",
            rusqlite::params![1_i64, now_utc(), checksum],
        )?;
    }
    if current < 2 {
        tx.execute_batch(SCHEMA_V2)
            .map_err(|e| CoreError::internal(InternalCode::Io, format!("schema v2: {e}")))?;
        let checksum = Digest::of(SCHEMA_V2.as_bytes()).to_hex();
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_utc, checksum) VALUES (?1, ?2, ?3)",
            rusqlite::params![2_i64, now_utc(), checksum],
        )?;
    }
    if current < 3 {
        tx.execute_batch(SCHEMA_V3)
            .map_err(|e| CoreError::internal(InternalCode::Io, format!("schema v3: {e}")))?;
        let checksum = Digest::of(SCHEMA_V3.as_bytes()).to_hex();
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_utc, checksum) VALUES (?1, ?2, ?3)",
            rusqlite::params![3_i64, now_utc(), checksum],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Highest applied schema version, or 0 if the schema table does not exist yet.
fn current_version(conn: &Connection) -> Result<i64> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok(0);
    }
    let v: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .map_err(CoreError::from)?;
    Ok(v.unwrap_or(0))
}

/// Minimal UTC timestamp string. Phase 1 uses a monotonic-free coarse clock; the
/// journal carries the precise monotonic value where ordering matters.
pub fn now_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

/// Monotonic nanoseconds since an arbitrary epoch (process start era). Used for
/// local elapsed ordering only; never subtracted across hosts (§18.10).
pub fn monotonic_ns() -> u128 {
    use std::time::Instant;
    // A process-lifetime-stable base so successive calls increase monotonically.
    thread_local! {
        static BASE: Instant = Instant::now();
    }
    BASE.with(|b| b.elapsed().as_nanos())
}
