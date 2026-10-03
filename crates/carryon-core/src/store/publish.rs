//! Two-phase content-addressed publish (spec §18.6/§19.1, §5 of the plan).
//!
//! Even though Phase 1 is local, the full digest/chunk/atomic-publish discipline
//! is used: stage bytes → verify whole-file digest → atomically rename into the
//! content-addressed path → commit metadata in one SQLite transaction. The
//! `objects`/`object_locations` rows become visible **only** after the file is in
//! place and the digest matched (CORE-004).

use super::Store;
use crate::db::migrations::now_utc;
use crate::db::Db;
use crate::error::{CoreError, Result, TransferCode};
use crate::ids::{Digest, TransferId};
use crate::journal::{EventType, Journal, JournalEvent};
use crate::model::object::Object;
use std::sync::atomic::{AtomicBool, Ordering};

/// Default chunk size (1 MiB, spec §18.6). Bounded; mobile may use smaller.
pub const DEFAULT_CHUNK_SIZE: u64 = 1024 * 1024;

/// A source of object bytes, addressed by (offset, length). The core drives
/// these reads and hashes the result — the byte source never writes the store.
pub trait ChunkSource {
    /// Return up to `length` bytes starting at `offset`. Fewer bytes than
    /// requested indicates end of object.
    fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>>;
}

/// Outcome of a publish: the final digest/location plus the transfer id used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOutcome {
    pub transfer: TransferId,
    pub digest: Digest,
    pub rel_path: String,
    /// True if the object was already present (publish was a no-op).
    pub deduplicated: bool,
}

/// Publish `declared` by pulling its bytes from `source`, verifying the digest,
/// and committing metadata. Journals `TRANSFER_BEGIN` before any bytes and
/// `TRANSFER_COMPLETE` only after the commit (CORE-005).
///
/// `chunk_size` defaults to [`DEFAULT_CHUNK_SIZE`] when `None`.
pub fn publish_object(
    db: &mut Db,
    store: &Store,
    journal: &mut Journal,
    declared: &Object,
    source: &dyn ChunkSource,
    chunk_size: Option<u64>,
) -> Result<PublishOutcome> {
    let transfer = TransferId::new();
    let chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE).max(1);
    let total = declared.logical_size;

    // Fast path: already published (NET-006 idempotency).
    if store.exists(&declared.content_hash) {
        commit_metadata(db, declared, &declared.content_hash.rel_path())?;
        return Ok(PublishOutcome {
            transfer,
            digest: declared.content_hash,
            rel_path: declared.content_hash.rel_path(),
            deduplicated: true,
        });
    }

    // 1. Record the transfer and journal the start before any bytes move.
    db.with_tx(|tx| {
        tx.execute(
            "INSERT INTO transfers (transfer_id, object_id, generation, content_hash, \
             total_length, chunk_size, state, staging_path, created_utc, updated_utc) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'staging', ?7, ?8, ?8)",
            rusqlite::params![
                transfer.to_string(),
                declared.id.0,
                declared.generation as i64,
                declared.content_hash.to_hex(),
                total as i64,
                chunk_size as i64,
                store
                    .layout()
                    .staging_dir()
                    .join(format!("{transfer}.tmp"))
                    .to_string_lossy(),
                now_utc(),
            ],
        )?;
        Ok(())
    })?;
    journal.append(
        &JournalEvent::new(EventType::TransferBegin, "OK").with_transfer(transfer.to_string()),
    )?;

    // 2-3. Stage bytes chunk by chunk; record per-chunk digests; reject a
    //      conflicting duplicate chunk (NET-007).
    let mut staged = store.stage_writer(transfer)?;
    let mut offset = 0u64;
    let mut index = 0i64;
    while offset < total {
        let want = chunk_size.min(total - offset);
        let bytes = source.read(offset, want)?;
        if bytes.is_empty() {
            break;
        }
        let d_i = Digest::of(&bytes).to_hex();

        // Conflicting duplicate detection for the same chunk index.
        let existing: Option<String> = db
            .conn()
            .query_row(
                "SELECT chunk_digest FROM transfer_chunks WHERE transfer_id=?1 AND chunk_index=?2",
                rusqlite::params![transfer.to_string(), index],
                |r| r.get(0),
            )
            .ok();
        if let Some(prev) = existing {
            if prev != d_i {
                fail_transfer(db, &transfer, "conflicting chunk")?;
                journal.append(
                    &JournalEvent::new(EventType::TransferInterrupted, "TRANSFER_ConflictingChunk")
                        .with_transfer(transfer.to_string()),
                )?;
                return Err(CoreError::transfer(
                    TransferCode::ConflictingChunk,
                    format!("chunk {index} digest differs from prior write"),
                ));
            }
        }

        staged.write_all(&bytes)?;
        staged.sync()?;
        let got = bytes.len() as i64;
        db.with_tx(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO transfer_chunks \
                 (transfer_id, chunk_index, chunk_digest, length, acked) VALUES (?1,?2,?3,?4,1)",
                rusqlite::params![transfer.to_string(), index, d_i, got],
            )?;
            Ok(())
        })?;
        offset += bytes.len() as u64;
        index += 1;
    }

    // 4. Verify + 5. atomic publish. A mismatch quarantines and commits nothing.
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE transfers SET state='verifying', updated_utc=?2 WHERE transfer_id=?1",
            rusqlite::params![transfer.to_string(), now_utc()],
        )?;
        Ok(())
    })?;
    let published = match store.verify_and_publish(staged, declared.content_hash) {
        Ok(p) => p,
        Err(e) => {
            fail_transfer(db, &transfer, "digest mismatch")?;
            journal.append(
                &JournalEvent::new(EventType::TransferInterrupted, "OBJECT_DigestMismatch")
                    .with_transfer(transfer.to_string()),
            )?;
            return Err(e);
        }
    };

    // 6. Commit metadata in one txn, then journal completion.
    commit_metadata(db, declared, &published.rel_path)?;
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE transfers SET state='published', updated_utc=?2 WHERE transfer_id=?1",
            rusqlite::params![transfer.to_string(), now_utc()],
        )?;
        Ok(())
    })?;
    journal.append(
        &JournalEvent::new(EventType::TransferComplete, "OK").with_transfer(transfer.to_string()),
    )?;

    Ok(PublishOutcome {
        transfer,
        digest: published.digest,
        rel_path: published.rel_path,
        deduplicated: false,
    })
}

/// Outcome of a resumable publish: finished, or cooperatively suspended with the
/// byte offset to resume from (§11.2). Suspension is a normal control-flow result,
/// not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishProgress {
    /// The object was fully staged, verified, and published.
    Completed(PublishOutcome),
    /// A cooperative suspend was requested mid-object. The staging file and the
    /// `transfers` row (state `'suspended'`, `acked_offset=next_offset`) persist so
    /// the transfer resumes from `next_offset`.
    Suspended {
        transfer: TransferId,
        next_offset: u64,
    },
}

/// Like [`publish_object`] but **resumable** and **suspendable** (§11.2/§19.3).
///
/// - Honors a prior `'suspended'` transfer for the same `content_hash`: reopens its
///   staging file, seeks to the recorded `acked_offset`, and resumes the chunk loop
///   there (byte-level resume — already-staged bytes are not re-pulled).
/// - Checks `suspend_flag` at each chunk boundary; if set, it flushes+syncs the
///   staging file, records `acked_offset`, sets state `'suspended'`, journals
///   `TransferInterrupted "SUSPENDED"` (distinct from the crash `"RECOVERED"`), and
///   returns [`PublishProgress::Suspended`] — publishing nothing yet.
///
/// A crash (process death) is NOT a suspend: a `'staging'`/`'verifying'` transfer is
/// still discarded by recovery; only an explicit suspend survives.
pub fn publish_object_resumable(
    db: &mut Db,
    store: &Store,
    journal: &mut Journal,
    declared: &Object,
    source: &dyn ChunkSource,
    chunk_size: Option<u64>,
    suspend_flag: &AtomicBool,
) -> Result<PublishProgress> {
    let chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE).max(1);
    let total = declared.logical_size;
    let hash_hex = declared.content_hash.to_hex();

    // Fast path: already published (NET-006 idempotency).
    if store.exists(&declared.content_hash) {
        commit_metadata(db, declared, &declared.content_hash.rel_path())?;
        return Ok(PublishProgress::Completed(PublishOutcome {
            transfer: TransferId::new(),
            digest: declared.content_hash,
            rel_path: declared.content_hash.rel_path(),
            deduplicated: true,
        }));
    }

    // Resume a prior suspended transfer for this content, or start a new one.
    let resumed: Option<(String, i64, String)> = db
        .conn()
        .query_row(
            "SELECT transfer_id, acked_offset, staging_path FROM transfers \
             WHERE content_hash=?1 AND state='suspended' LIMIT 1",
            rusqlite::params![hash_hex],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .ok();

    let (transfer, mut offset, mut index, mut staged) = match resumed {
        Some((tid_str, acked, staging_path)) => {
            let transfer = TransferId(uuid::Uuid::parse_str(&tid_str).map_err(|_| {
                CoreError::transfer(TransferCode::ResumeFailure, "bad transfer id")
            })?);
            let staged =
                store.reopen_stage_writer(transfer, std::path::Path::new(&staging_path))?;
            let offset = acked.max(0) as u64;
            // Count already-acked chunks to continue chunk_index numbering.
            let index: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM transfer_chunks WHERE transfer_id=?1",
                    rusqlite::params![tid_str],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            db.with_tx(|tx| {
                tx.execute(
                    "UPDATE transfers SET state='staging', updated_utc=?2 WHERE transfer_id=?1",
                    rusqlite::params![tid_str, now_utc()],
                )?;
                Ok(())
            })?;
            (transfer, offset, index, staged)
        }
        None => {
            let transfer = TransferId::new();
            db.with_tx(|tx| {
                tx.execute(
                    "INSERT INTO transfers (transfer_id, object_id, generation, content_hash, \
                     total_length, chunk_size, state, staging_path, acked_offset, created_utc, updated_utc) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'staging', ?7, 0, ?8, ?8)",
                    rusqlite::params![
                        transfer.to_string(),
                        declared.id.0,
                        declared.generation as i64,
                        hash_hex,
                        total as i64,
                        chunk_size as i64,
                        store
                            .layout()
                            .staging_dir()
                            .join(format!("{transfer}.tmp"))
                            .to_string_lossy(),
                        now_utc(),
                    ],
                )?;
                Ok(())
            })?;
            journal.append(
                &JournalEvent::new(EventType::TransferBegin, "OK")
                    .with_transfer(transfer.to_string()),
            )?;
            let staged = store.stage_writer(transfer)?;
            (transfer, 0u64, 0i64, staged)
        }
    };

    // Stage chunk by chunk, polling the suspend flag at each boundary.
    while offset < total {
        if suspend_flag.load(Ordering::SeqCst) {
            staged.sync()?;
            db.with_tx(|tx| {
                tx.execute(
                    "UPDATE transfers SET state='suspended', acked_offset=?2, updated_utc=?3 \
                     WHERE transfer_id=?1",
                    rusqlite::params![transfer.to_string(), offset as i64, now_utc()],
                )?;
                Ok(())
            })?;
            journal.append(
                &JournalEvent::new(EventType::TransferInterrupted, "SUSPENDED")
                    .with_transfer(transfer.to_string()),
            )?;
            return Ok(PublishProgress::Suspended {
                transfer,
                next_offset: offset,
            });
        }

        let want = chunk_size.min(total - offset);
        let bytes = source.read(offset, want)?;
        if bytes.is_empty() {
            break;
        }
        let d_i = Digest::of(&bytes).to_hex();

        let existing: Option<String> = db
            .conn()
            .query_row(
                "SELECT chunk_digest FROM transfer_chunks WHERE transfer_id=?1 AND chunk_index=?2",
                rusqlite::params![transfer.to_string(), index],
                |r| r.get(0),
            )
            .ok();
        if let Some(prev) = existing {
            if prev != d_i {
                fail_transfer(db, &transfer, "conflicting chunk")?;
                journal.append(
                    &JournalEvent::new(EventType::TransferInterrupted, "TRANSFER_ConflictingChunk")
                        .with_transfer(transfer.to_string()),
                )?;
                return Err(CoreError::transfer(
                    TransferCode::ConflictingChunk,
                    format!("chunk {index} digest differs from prior write"),
                ));
            }
        }

        staged.write_all(&bytes)?;
        staged.sync()?;
        let got = bytes.len() as i64;
        db.with_tx(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO transfer_chunks \
                 (transfer_id, chunk_index, chunk_digest, length, acked) VALUES (?1,?2,?3,?4,1)",
                rusqlite::params![transfer.to_string(), index, d_i, got],
            )?;
            Ok(())
        })?;
        offset += bytes.len() as u64;
        index += 1;
    }

    // Verify + atomic publish (same discipline as publish_object).
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE transfers SET state='verifying', acked_offset=?2, updated_utc=?3 WHERE transfer_id=?1",
            rusqlite::params![transfer.to_string(), offset as i64, now_utc()],
        )?;
        Ok(())
    })?;
    let published = match store.verify_and_publish(staged, declared.content_hash) {
        Ok(p) => p,
        Err(e) => {
            fail_transfer(db, &transfer, "digest mismatch")?;
            journal.append(
                &JournalEvent::new(EventType::TransferInterrupted, "OBJECT_DigestMismatch")
                    .with_transfer(transfer.to_string()),
            )?;
            return Err(e);
        }
    };

    commit_metadata(db, declared, &published.rel_path)?;
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE transfers SET state='published', updated_utc=?2 WHERE transfer_id=?1",
            rusqlite::params![transfer.to_string(), now_utc()],
        )?;
        Ok(())
    })?;
    journal.append(
        &JournalEvent::new(EventType::TransferComplete, "OK").with_transfer(transfer.to_string()),
    )?;

    Ok(PublishProgress::Completed(PublishOutcome {
        transfer,
        digest: published.digest,
        rel_path: published.rel_path,
        deduplicated: false,
    }))
}

/// Insert the `objects` + `object_locations` rows in one transaction. Called only
/// after the file exists at its final path and the digest matched.
fn commit_metadata(db: &mut Db, obj: &Object, rel_path: &str) -> Result<()> {
    let parents = serde_json::to_string(&obj.parents)?;
    db.with_tx(|tx| {
        tx.execute(
            "INSERT OR IGNORE INTO object_locations (content_hash, rel_path, present, verified_utc) \
             VALUES (?1, ?2, 1, ?3)",
            rusqlite::params![obj.content_hash.to_hex(), rel_path, now_utc()],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO objects \
             (object_id, generation, kind, schema_id, content_hash, wire_hash, logical_size, \
              wire_size, recipe_id, portable, sensitivity, retention, parents_json, created_utc) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            rusqlite::params![
                obj.id.0,
                obj.generation as i64,
                obj.kind.as_str(),
                obj.schema_id,
                obj.content_hash.to_hex(),
                obj.wire_hash.map(|d| d.to_hex()),
                obj.logical_size as i64,
                obj.wire_size.map(|s| s as i64),
                obj.recipe_id,
                obj.portable as i64,
                obj.sensitivity.as_str(),
                retention_tag(&obj.retention),
                parents,
                now_utc(),
            ],
        )?;
        Ok(())
    })
}

fn fail_transfer(db: &mut Db, transfer: &TransferId, _why: &str) -> Result<()> {
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE transfers SET state='failed', updated_utc=?2 WHERE transfer_id=?1",
            rusqlite::params![transfer.to_string(), now_utc()],
        )?;
        Ok(())
    })
}

fn retention_tag(r: &crate::model::object::Retention) -> String {
    use crate::model::object::Retention::*;
    match r {
        Session => "session".into(),
        Bounded(n) => format!("bounded:{n}"),
        Persistent => "persistent".into(),
        NoCache => "no_cache".into(),
    }
}
