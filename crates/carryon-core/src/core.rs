//! The `Core` handle (spec §17): the local launcher API as blocking Rust
//! functions. Phase 1 is a Rust API, not a network server.
//!
//! `Core::open` creates the data directory, opens the metadata DB (running
//! migrations), opens and repairs the journal, and runs crash recovery (§19.3)
//! before returning.

use crate::db::migrations::now_utc;
use crate::db::Db;
use crate::error::{AdapterCode, CoreError, Result};
use crate::host::{consent, AdapterHost};
use crate::ids::{Epoch, SessionId};
use crate::journal::{EventType, Journal, JournalEvent};
use crate::model::{AuthorityMode, Sensitivity, Session, SessionState};
use crate::recovery::{self, RecoveryReport};
use crate::store::Store;
use carryon_adapter_api::{Adapter, AdapterInfo, ConsentScope, ConsentToken};
use std::path::Path;

/// Request to create a session.
#[derive(Debug, Clone)]
pub struct CreateSessionReq {
    pub adapter_id: String,
    pub title: String,
    pub privacy: Sensitivity,
    pub authority_mode: AuthorityMode,
}

/// The engine core handle.
pub struct Core {
    db: Db,
    store: Store,
    journal: Journal,
    journal_path: std::path::PathBuf,
    host: AdapterHost,
    last_recovery: RecoveryReport,
    /// Transfer chunk size override (`None` = [`crate::store::DEFAULT_CHUNK_SIZE`]).
    /// Mobile sets this small under memory/background constraints (§18.6).
    chunk_size: Option<u64>,
    /// Active preparation budget enforced on import (§6.8).
    budget: crate::model::Budget,
    /// Whether the app is foreground. Background shrinks the effective budget
    /// (foreground-first, §20.2). Starts `true`.
    foreground: bool,
    /// Cooperative suspend flag read at chunk boundaries during a transfer
    /// (§11.2 survive suspension). Set from the shell's suspend handler thread
    /// via [`Core::request_suspend`]; the sole thread-safe cross-thread entry.
    suspend_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Bounds on a settable transfer chunk size (§18.6 bounded, mobile-friendly).
const MIN_CHUNK_SIZE: u64 = 4 * 1024;
const MAX_CHUNK_SIZE: u64 = 8 * 1024 * 1024;

impl Core {
    /// Open (or create) an engine rooted at `data_dir`. Runs migrations, repairs
    /// the journal, and performs crash recovery (§19.3).
    pub fn open(data_dir: &Path) -> Result<Core> {
        std::fs::create_dir_all(data_dir)?;
        let db = Db::open(&data_dir.join("metadata.sqlite3"))?;
        let store = Store::open(data_dir)?;
        std::fs::create_dir_all(data_dir.join("journals"))?;
        let journal_path = data_dir.join("journals/events.log");
        let (journal, _events, truncated) = Journal::open(&journal_path)?;

        let mut core = Core {
            db,
            store,
            journal,
            journal_path,
            host: AdapterHost::new(),
            last_recovery: RecoveryReport::default(),
            chunk_size: None,
            budget: crate::model::Budget::local_default(),
            foreground: true,
            suspend_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        core.last_recovery =
            recovery::recover(&mut core.db, &core.store, &mut core.journal, truncated)?;
        Ok(core)
    }

    /// Open a fresh engine (no recovery; empty dir).
    pub fn create(data_dir: &Path) -> Result<Core> {
        Self::open(data_dir)
    }

    /// The report from the recovery pass run at open (§19.3.8).
    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.last_recovery
    }

    // --- Phase 2+ transfer tuning (§18.6/§6.8/§11.2) ---

    /// Set the transfer chunk size in bytes (§18.6). `None` resets to the
    /// default. A value outside `[4 KiB, 8 MiB]` is rejected (bounded).
    pub fn set_chunk_size(&mut self, bytes: Option<u64>) -> Result<()> {
        if let Some(n) = bytes {
            if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&n) {
                return Err(CoreError::transfer(
                    crate::error::TransferCode::Quota,
                    format!("chunk size {n} outside [{MIN_CHUNK_SIZE}, {MAX_CHUNK_SIZE}]"),
                ));
            }
        }
        self.chunk_size = bytes;
        Ok(())
    }

    /// The effective chunk size override passed to `publish_object`.
    pub(crate) fn chunk_size(&self) -> Option<u64> {
        self.chunk_size
    }

    /// Set the active preparation budget enforced on import (§6.8).
    pub fn set_budget(&mut self, budget: crate::model::Budget) {
        self.budget = budget;
    }

    /// Set foreground/background. Background shrinks the effective budget so a
    /// large import is admitted only in the foreground (foreground-first, §20.2).
    pub fn set_foreground(&mut self, foreground: bool) {
        self.foreground = foreground;
    }

    /// The budget to enforce right now: the active budget in foreground, or the
    /// smaller of it and a conservative background budget when backgrounded.
    pub(crate) fn effective_budget(&self) -> crate::model::Budget {
        if self.foreground {
            self.budget
        } else {
            crate::model::Budget::background_floor(&self.budget)
        }
    }

    /// Request cooperative suspension of an in-flight transfer (§11.2). Thread-safe;
    /// the running (blocking) import observes it at the next chunk boundary. This is
    /// the ONLY method safe to call from another thread while an import runs.
    pub fn request_suspend(&self) {
        self.suspend_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Clear the suspend flag (before starting or resuming a transfer).
    pub(crate) fn clear_suspend(&self) {
        self.suspend_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// A clone of the suspend flag handle for the transfer loop to poll.
    pub(crate) fn suspend_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.suspend_flag.clone()
    }

    /// Total recorded transfer chunks across all transfers (observability; lets a
    /// caller confirm small-chunk behavior, §18.6).
    pub fn transfer_chunk_count(&self) -> u64 {
        self.db
            .conn()
            .query_row("SELECT COUNT(*) FROM transfer_chunks", [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n.max(0) as u64)
            .unwrap_or(0)
    }

    // --- internal accessors for sibling modules (prepare, evidence) ---

    pub(crate) fn db(&mut self) -> &mut Db {
        &mut self.db
    }
    pub(crate) fn db_shared(&self) -> &Db {
        &self.db
    }
    pub(crate) fn store(&self) -> &Store {
        &self.store
    }
    pub(crate) fn journal(&mut self) -> &mut Journal {
        &mut self.journal
    }
    pub(crate) fn host(&mut self) -> &mut AdapterHost {
        &mut self.host
    }
    pub(crate) fn journal_path(&self) -> &std::path::Path {
        &self.journal_path
    }
    pub(crate) fn parts(&mut self) -> (&mut Db, &Store, &mut Journal, &mut AdapterHost) {
        (&mut self.db, &self.store, &mut self.journal, &mut self.host)
    }

    // --- §17.2 adapters ---

    /// Register a compiled-in adapter (verifies its manifest first) and record it.
    pub fn register_adapter(&mut self, adapter: Box<dyn Adapter>) -> Result<AdapterInfo> {
        let info = self.host.register(adapter)?;
        let manifest_json = serde_json::to_string(&info)?;
        let digest = crate::ids::Digest::of(manifest_json.as_bytes()).to_hex();
        let level = format!("{:?}", info.integration_level);
        self.db.with_tx(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO adapters \
                 (adapter_id, adapter_version, integration_level, manifest_json, manifest_digest, registered_utc) \
                 VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    info.adapter_id,
                    info.adapter_version,
                    level,
                    manifest_json,
                    digest,
                    now_utc()
                ],
            )?;
            Ok(())
        })?;
        self.journal.append(
            &JournalEvent::new(EventType::AdapterRegistered, "OK")
                .with_metadata(serde_json::json!({ "adapter_id": info.adapter_id })),
        )?;
        Ok(info)
    }

    pub fn list_adapters(&self) -> Vec<AdapterInfo> {
        self.host.list()
    }

    pub fn get_adapter(&self, id: &str) -> Option<AdapterInfo> {
        self.host.info(id)
    }

    /// Grant consent for an adapter scope (§10.6).
    pub fn grant_adapter_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken> {
        if self.host.info(&scope.adapter_id).is_none() {
            return Err(CoreError::adapter(
                AdapterCode::Missing,
                format!("adapter '{}' not registered", scope.adapter_id),
            ));
        }
        let token = consent::grant(&mut self.db, &scope)?;
        self.journal.append(
            &JournalEvent::new(EventType::ConsentGranted, "OK")
                .with_metadata(serde_json::json!({ "adapter_id": scope.adapter_id })),
        )?;
        Ok(token)
    }

    pub fn revoke_adapter_consent(&mut self, token: &ConsentToken) -> Result<()> {
        consent::revoke(&mut self.db, token)?;
        self.journal
            .append(&JournalEvent::new(EventType::ConsentRevoked, "OK"))?;
        Ok(())
    }

    /// Whether a consent token is live for an adapter.
    pub fn consent_live(&self, adapter_id: &str, token: &ConsentToken) -> Result<bool> {
        consent::is_live(&self.db, adapter_id, token)
    }

    // --- §17.3 sessions (creation here; cut/action in prepare.rs) ---

    /// Create a session bound to a registered adapter and open its first
    /// authority epoch.
    pub fn create_session(&mut self, req: CreateSessionReq) -> Result<SessionId> {
        let info = self.host.info(&req.adapter_id).ok_or_else(|| {
            CoreError::adapter(
                AdapterCode::Missing,
                format!("adapter '{}' not registered", req.adapter_id),
            )
        })?;
        let id = SessionId::new();
        let epoch = Epoch(0);
        self.db.with_tx(|tx| {
            tx.execute(
                "INSERT INTO sessions \
                 (session_id, adapter_id, adapter_version, schema_version, title, creation_device, \
                  created_utc, authority_epoch, latest_cut, privacy, manifest_root, state, generation) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9,NULL,?10,0)",
                rusqlite::params![
                    id.to_string(),
                    info.adapter_id,
                    info.adapter_version,
                    1_i64,
                    req.title,
                    "local",
                    now_utc(),
                    epoch.0 as i64,
                    req.privacy.as_str(),
                    SessionState::Idle.as_str(),
                ],
            )?;
            tx.execute(
                "INSERT INTO authority_epochs (session_id, epoch, owner_device, mode, opened_utc) \
                 VALUES (?1, ?2, 'local', ?3, ?4)",
                rusqlite::params![
                    id.to_string(),
                    epoch.0 as i64,
                    req.authority_mode.as_str(),
                    now_utc()
                ],
            )?;
            Ok(())
        })?;
        self.journal.append(
            &JournalEvent::new(EventType::SessionCreated, "OK").with_session(id.to_string()),
        )?;
        Ok(id)
    }

    /// Fetch a session by id.
    pub fn get_session(&self, id: SessionId) -> Option<Session> {
        self.db
            .conn()
            .query_row(
                "SELECT adapter_id, adapter_version, schema_version, title, creation_device, \
                 created_utc, authority_epoch, latest_cut, privacy, state, generation \
                 FROM sessions WHERE session_id=?1",
                rusqlite::params![id.to_string()],
                |r| {
                    Ok(Session {
                        id,
                        adapter_id: r.get(0)?,
                        adapter_version: r.get(1)?,
                        schema_version: r.get::<_, i64>(2)? as u32,
                        title: r.get(3)?,
                        creation_device: r.get(4)?,
                        created_utc: r.get(5)?,
                        authority_epoch: Epoch(r.get::<_, i64>(6)? as u64),
                        latest_cut: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                        privacy: parse_sensitivity(&r.get::<_, String>(8)?),
                        devices: vec!["local".into()],
                        manifest_root: None,
                        state: SessionState::parse_tag(&r.get::<_, String>(9)?)
                            .unwrap_or(SessionState::Idle),
                        generation: r.get::<_, i64>(10)? as u64,
                    })
                },
            )
            .ok()
    }

    /// List all session ids.
    pub fn list_sessions(&self) -> Vec<SessionId> {
        let conn = self.db.conn();
        let mut stmt = match conn.prepare("SELECT session_id FROM sessions") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .and_then(|it| it.collect::<std::result::Result<Vec<_>, _>>())
            .unwrap_or_default();
        rows.into_iter()
            .filter_map(|s| uuid::Uuid::parse_str(&s).ok().map(SessionId))
            .collect()
    }
}

fn parse_sensitivity(s: &str) -> Sensitivity {
    match s {
        "public" => Sensitivity::Public,
        "personal" => Sensitivity::Personal,
        "confidential" => Sensitivity::Confidential,
        "secret" => Sensitivity::Secret,
        "prohibited" => Sensitivity::Prohibited,
        _ => Sensitivity::Public,
    }
}
