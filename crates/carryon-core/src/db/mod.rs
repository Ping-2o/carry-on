//! SQLite wrapper (spec §8.4/§19.2). WAL on local filesystems; `synchronous=FULL`
//! so the two-phase object-publish commit is durable.

pub mod migrations;
pub mod schema;

use crate::error::{CoreError, InternalCode, Result};
use rusqlite::Connection;
use std::path::Path;

/// Owns the SQLite connection and applies migrations on open.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (or create) the metadata database at `path` and migrate it.
    pub fn open(path: &Path) -> Result<Db> {
        let mut conn = Connection::open(path)?;
        // WAL is safe on local filesystems only (§8.4); FULL durability for the
        // commit rows that gate object visibility.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::migrate(&mut conn)?;
        Ok(Db { conn })
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory() -> Result<Db> {
        let mut conn = Connection::open_in_memory()?;
        migrations::migrate(&mut conn)?;
        Ok(Db { conn })
    }

    /// Borrow the underlying connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Mutable borrow for transactions.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Run `f` inside a transaction, committing on `Ok` and rolling back on `Err`.
    pub fn with_tx<T>(&mut self, f: impl FnOnce(&rusqlite::Transaction) -> Result<T>) -> Result<T> {
        let tx = self.conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// `PRAGMA integrity_check` (§19.3.1). Returns `Ok(())` only if "ok".
    pub fn integrity_check(&self) -> Result<()> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if result == "ok" {
            Ok(())
        } else {
            Err(CoreError::internal(
                InternalCode::DbCorrupt,
                format!("integrity_check: {result}"),
            ))
        }
    }
}
