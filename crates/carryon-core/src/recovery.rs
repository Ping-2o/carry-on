//! Crash recovery (spec §19.3). Run inside `Core::open` after migrations. Never
//! exposes incomplete staged bytes as valid (CORE-004) and never relabels an
//! interrupted operation as success (§19.3.9, EVD-003).

use crate::db::migrations::now_utc;
use crate::db::Db;
use crate::error::Result;
use crate::ids::Digest;
use crate::journal::{EventType, Journal, JournalEvent};
use crate::store::Store;

/// Summary of what recovery found and did, surfaced to the caller/UI (§19.3.8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub interrupted_transfers: Vec<String>,
    pub hidden_incomplete_objects: Vec<String>,
    pub ambiguous_sessions: Vec<String>,
    pub journal_truncated: bool,
}

impl RecoveryReport {
    pub fn is_clean(&self) -> bool {
        self.interrupted_transfers.is_empty()
            && self.hidden_incomplete_objects.is_empty()
            && self.ambiguous_sessions.is_empty()
            && !self.journal_truncated
    }
}

/// Run the §19.3 recovery sequence. `journal` has already been opened (which
/// performs step 7's tail truncation); `journal_truncated` reflects whether the
/// recovered byte length was shorter than the file (detected by the caller).
pub fn recover(
    db: &mut Db,
    store: &Store,
    journal: &mut Journal,
    journal_truncated: bool,
) -> Result<RecoveryReport> {
    let mut report = RecoveryReport {
        journal_truncated,
        ..Default::default()
    };

    // 1. Integrity check.
    db.integrity_check()?;

    // 2-3. Resolve transfers left mid-flight.
    let pending: Vec<(String, String, String)> = {
        let conn = db.conn();
        let mut stmt = conn.prepare(
            "SELECT transfer_id, state, content_hash FROM transfers \
             WHERE state IN ('staging','verifying')",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    for (tid, _state, hash_hex) in pending {
        let digest = Digest::from_hex(&hash_hex);
        let already_published = digest.map(|d| store.exists(&d)).unwrap_or(false);
        if already_published {
            // Verified file already in place: finish the commit idempotently.
            db.with_tx(|tx| {
                tx.execute(
                    "UPDATE transfers SET state='published', updated_utc=?2 WHERE transfer_id=?1",
                    rusqlite::params![tid, now_utc()],
                )?;
                Ok(())
            })?;
            journal.append(
                &JournalEvent::new(EventType::TransferComplete, "RECOVERED")
                    .with_transfer(tid.clone()),
            )?;
        } else {
            // Discard staging bytes; mark failed. Never exposed.
            let staging = store.layout().staging_dir().join(format!("{tid}.tmp"));
            let _ = std::fs::remove_file(&staging);
            db.with_tx(|tx| {
                tx.execute(
                    "UPDATE transfers SET state='failed', updated_utc=?2 WHERE transfer_id=?1",
                    rusqlite::params![tid, now_utc()],
                )?;
                Ok(())
            })?;
            journal.append(
                &JournalEvent::new(EventType::TransferInterrupted, "RECOVERED")
                    .with_transfer(tid.clone()),
            )?;
            report.interrupted_transfers.push(tid);
        }
    }

    // 2b. Graceful-suspend survival (§11.2): a 'suspended' transfer is NOT a crash
    //     casualty — keep it resumable. Only demote to 'failed' if its staging file
    //     vanished (then it is honestly unresumable, reported).
    let suspended: Vec<(String, String)> = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT transfer_id, staging_path FROM transfers WHERE state='suspended'")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    for (tid, staging_path) in suspended {
        if std::path::Path::new(&staging_path).exists() {
            // Resumable: leave the row at 'suspended'. Nothing to do.
            continue;
        }
        db.with_tx(|tx| {
            tx.execute(
                "UPDATE transfers SET state='failed', updated_utc=?2 WHERE transfer_id=?1",
                rusqlite::params![tid, now_utc()],
            )?;
            Ok(())
        })?;
        journal.append(
            &JournalEvent::new(EventType::TransferInterrupted, "SUSPEND_STAGING_LOST")
                .with_transfer(tid.clone()),
        )?;
        report.interrupted_transfers.push(tid);
    }

    // 4. No object_locations.present=1 may lack a verified file.
    let present: Vec<(String, String)> = {
        let conn = db.conn();
        let mut stmt =
            conn.prepare("SELECT content_hash, rel_path FROM object_locations WHERE present=1")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    for (hash_hex, rel) in present {
        let path = store.layout().object_path(&rel);
        let ok = path.exists()
            && Digest::from_hex(&hash_hex)
                .map(|d| store.exists(&d))
                .unwrap_or(false);
        if !ok {
            db.with_tx(|tx| {
                tx.execute(
                    "UPDATE object_locations SET present=0 WHERE content_hash=?1",
                    rusqlite::params![hash_hex],
                )?;
                Ok(())
            })?;
            journal.append(&JournalEvent::new(
                EventType::ObjectIncompleteHidden,
                "RECOVERED",
            ))?;
            report.hidden_incomplete_objects.push(hash_hex);
        }
    }

    // 5-6. Ambiguous authority: sessions with an ambiguous epoch block writes.
    let ambiguous: Vec<String> = {
        let conn = db.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT session_id FROM authority_epochs WHERE ambiguous=1 AND closed_utc IS NULL",
        )?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    for s in &ambiguous {
        journal.append(
            &JournalEvent::new(EventType::AuthorityAmbiguous, "RECOVERED").with_session(s.clone()),
        )?;
    }
    report.ambiguous_sessions = ambiguous;

    // 9. Any non-terminal session left by a crash is Inconclusive, never success.
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE sessions SET state='Inconclusive' \
             WHERE state IN ('Preparing','CutProposed','Importing')",
            [],
        )?;
        Ok(())
    })?;

    // 8. Record the recovery report in the journal.
    journal.append(
        &JournalEvent::new(EventType::RecoveryReport, "OK").with_metadata(serde_json::json!({
            "interrupted_transfers": report.interrupted_transfers.len(),
            "hidden_incomplete_objects": report.hidden_incomplete_objects.len(),
            "ambiguous_sessions": report.ambiguous_sessions.len(),
            "journal_truncated": report.journal_truncated,
        })),
    )?;

    Ok(report)
}
