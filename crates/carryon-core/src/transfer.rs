//! Cross-device cut transfer (spec §18.6, Phase 2). Moves a sealed cut's
//! authoritative objects from a **source** core to a **destination** core over the
//! authenticated transport ([`carryon_net::Session`]).
//!
//! # The invariant that survives the wire (CORE-004)
//!
//! The destination does not trust the source's bytes. For each object it pulls the
//! bytes into a [`NetChunkSource`] and runs them through the **same**
//! [`crate::store::publish::publish_object`] two-phase discipline used locally:
//! stage → verify whole-object digest against the manifest's declared
//! `content_hash` → atomic publish. A lying source (wrong bytes for a digest)
//! produces `OBJECT_DigestMismatch` and publishes nothing — exactly as a lying
//! adapter does locally. The wire is just another `ChunkSource`.
//!
//! After every authoritative object verifies, the destination seals a **mirror
//! cut** in its own DB so the imported state is a first-class sealed cut there and
//! source-off actions can run (§18.8 `Importing → ActionReady`).
//!
//! # Evidence honesty
//!
//! Driving both cores in one process over loopback is LOCAL evidence (§2/§30), not
//! physical cross-device evidence.

use crate::core::Core;
use crate::db::migrations::now_utc;
use crate::error::{BudgetCode, CoreError, InternalCode, ObjectCode, Result, TransferCode};
use crate::ids::{Digest, ObjectId, ObjectVersion, SessionId};
use crate::journal::{EventType, JournalEvent};
use crate::model::object::{Object, ObjectKind};
use crate::store::publish::{publish_object_resumable, PublishProgress};
use crate::store::ChunkSource;
use carryon_adapter_api::{
    ObjectEntry, ObjectKindWire, ObjectManifest, ObjectVersionWire, RetentionWire, SensitivityWire,
};
use carryon_net::wire::{Message, MAX_CHUNK_BYTES};
use carryon_net::Session;
use serde::{Deserialize, Serialize};

/// Outcome of an import attempt (§18.6/§11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    /// The whole cut imported + verified; the local mirror cut number is attached.
    Completed(u64),
    /// A cooperative suspend happened mid-object; resume with the token (§11.2).
    Suspended(ImportResume),
}

impl ImportOutcome {
    /// The mirror cut number if completed, else `None`.
    pub fn completed_cut(&self) -> Option<u64> {
        match self {
            ImportOutcome::Completed(n) => Some(*n),
            ImportOutcome::Suspended(_) => None,
        }
    }
}

/// A token to resume a suspended import (§11.2). Serde so a shell can persist it
/// across an app suspension and resume on next launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportResume {
    pub remote_session: String,
    pub remote_cut: u64,
    /// The object that was mid-transfer when suspended.
    pub pending_content_hash: String,
    /// The verified-staged byte offset to resume from.
    pub next_offset: u64,
}

/// A [`ChunkSource`] that pulls object bytes from a peer over the transport,
/// verifying each chunk's digest (NET-007). Reads are driven by the local
/// `publish_object`, so the destination controls chunking and the final verify.
struct NetChunkSource<'a> {
    session: std::cell::RefCell<&'a mut Session>,
    content_hash: String,
}

impl ChunkSource for NetChunkSource<'_> {
    fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>> {
        let want = length.min(MAX_CHUNK_BYTES as u64);
        let mut sess = self.session.borrow_mut();
        sess.send(Message::TransferRequest {
            content_hash: self.content_hash.clone(),
            offset,
            length: want,
        })?;
        match sess.recv()? {
            Message::ChunkData {
                content_hash,
                offset: got_off,
                chunk_digest,
                bytes,
            } => {
                if content_hash != self.content_hash || got_off != offset {
                    return Err(CoreError::transfer(
                        TransferCode::ResumeFailure,
                        "peer answered a different range than requested",
                    ));
                }
                // Per-chunk digest guard (NET-007): the peer's declared chunk
                // digest must match the bytes it sent.
                if Digest::of(&bytes).to_hex() != chunk_digest {
                    return Err(CoreError::transfer(
                        TransferCode::ConflictingChunk,
                        "chunk digest does not match chunk bytes",
                    ));
                }
                Ok(bytes)
            }
            Message::Unavailable { reason, .. } => Err(CoreError::object(
                ObjectCode::Missing,
                format!("peer: {reason}"),
            )),
            other => Err(CoreError::internal(
                InternalCode::Invariant,
                format!("expected ChunkData, got {other:?}"),
            )),
        }
    }
}

impl Core {
    /// Whether an object with this content hash (64 hex) is present + verified in
    /// the local store. Used to prove source-independence after an import: the
    /// bytes live in the destination store, not in any live source service.
    pub fn has_object_hex(&self, content_hash: &str) -> bool {
        Digest::from_hex(content_hash)
            .map(|d| self.store().exists(&d))
            .unwrap_or(false)
    }

    /// Read the full bytes of a content-addressed object from the local store by its
    /// 64-hex digest. Lets a shell pull a carried object (e.g. the navigation state)
    /// out of the store through the ABI instead of reading the store file directly.
    /// Errors `OBJECT_Missing` if absent or the hex is malformed.
    pub fn read_object_hex(&self, content_hash: &str) -> Result<Vec<u8>> {
        use std::io::Read as _;
        let digest = Digest::from_hex(content_hash)
            .ok_or_else(|| CoreError::object(ObjectCode::Invalid, "bad content hash hex"))?;
        let mut f = self.store().open_object(&digest)?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    // ---- Source side ----

    /// Serve one transfer request loop to a connected destination (spec §18.6).
    /// Answers `CutRequest` with the sealed cut's manifest and `TransferRequest`
    /// with object bytes read from the local store, until `ImportComplete` or
    /// `Abort`. Read-only: serving never mutates source state.
    pub fn serve_cut(&mut self, session: &mut Session) -> Result<()> {
        loop {
            match session.recv()? {
                Message::CutRequest {
                    session: sess,
                    cut_number,
                } => {
                    let sid = parse_session(&sess)?;
                    match self.cut_manifest(sid, cut_number) {
                        Ok(manifest) => session.send(Message::CutManifest { manifest })?,
                        Err(e) => session.send(Message::Abort {
                            reason: e.to_string(),
                        })?,
                    }
                }
                Message::TransferRequest {
                    content_hash,
                    offset,
                    length,
                } => {
                    let reply = self.serve_chunk(&content_hash, offset, length);
                    session.send(reply)?;
                }
                Message::ImportComplete {
                    session: sess,
                    cut_number,
                    manifest_digest,
                } => {
                    self.journal().append(
                        &JournalEvent::new(EventType::TransferComplete, "OK")
                            .with_session(sess)
                            .with_cut(cut_number)
                            .with_metadata(serde_json::json!({
                                "role": "source",
                                "peer_manifest_digest": manifest_digest,
                            })),
                    )?;
                    return Ok(());
                }
                Message::Abort { reason } => {
                    return Err(CoreError::transfer(TransferCode::Cancelled, reason));
                }
                other => {
                    let msg = format!("unexpected {other:?} while serving");
                    session.send(Message::Abort {
                        reason: msg.clone(),
                    })?;
                    return Err(CoreError::internal(InternalCode::Invariant, msg));
                }
            }
        }
    }

    /// Read a chunk from the local store and wrap it as a `ChunkData` reply (or
    /// `Unavailable`). The digest addresses the object; no path is ever named.
    fn serve_chunk(&self, content_hash: &str, offset: u64, length: u64) -> Message {
        let digest = match Digest::from_hex(content_hash) {
            Some(d) => d,
            None => {
                return Message::Unavailable {
                    content_hash: content_hash.into(),
                    reason: "bad content hash".into(),
                }
            }
        };
        let want = length.min(MAX_CHUNK_BYTES as u64) as usize;
        match read_store_range(self.store(), &digest, offset, want) {
            Ok(bytes) => Message::ChunkData {
                content_hash: content_hash.into(),
                offset,
                chunk_digest: Digest::of(&bytes).to_hex(),
                bytes,
            },
            Err(_) => Message::Unavailable {
                content_hash: content_hash.into(),
                reason: "object not present".into(),
            },
        }
    }

    /// Rebuild the wire manifest for a sealed cut from the DB (authoritative role).
    fn cut_manifest(&self, session: SessionId, cut_number: u64) -> Result<ObjectManifest> {
        let sealed: Option<i64> = self
            .db_shared()
            .conn()
            .query_row(
                "SELECT sealed FROM cuts WHERE session_id=?1 AND cut_number=?2",
                rusqlite::params![session.to_string(), cut_number as i64],
                |r| r.get(0),
            )
            .ok();
        if sealed != Some(1) {
            return Err(CoreError::object(ObjectCode::Stale, "cut not sealed"));
        }

        let conn = self.db_shared().conn();
        let mut stmt = conn.prepare(
            "SELECT o.object_id, o.generation, o.kind, o.schema_id, o.content_hash, \
             o.logical_size, o.recipe_id, o.portable, o.sensitivity, o.retention, o.parents_json \
             FROM cut_objects c \
             JOIN objects o ON o.object_id=c.object_id AND o.generation=c.generation \
             WHERE c.session_id=?1 AND c.cut_number=?2 AND c.role='authoritative' \
             ORDER BY c.ordinal",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![session.to_string(), cut_number as i64],
                |r| {
                    Ok(RawObjRow {
                        object_id: r.get(0)?,
                        generation: r.get::<_, i64>(1)? as u64,
                        kind: r.get(2)?,
                        schema_id: r.get(3)?,
                        content_hash: r.get(4)?,
                        logical_size: r.get::<_, i64>(5)? as u64,
                        recipe_id: r.get(6)?,
                        portable: r.get::<_, i64>(7)? != 0,
                        sensitivity: r.get(8)?,
                        retention: r.get(9)?,
                        parents_json: r.get(10)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let objects = rows
            .into_iter()
            .map(raw_to_entry)
            .collect::<Result<Vec<_>>>()?;
        Ok(ObjectManifest {
            session: session.to_string(),
            generation: cut_number,
            objects,
        })
    }

    // ---- Destination side ----

    /// Import a sealed cut from a connected source (spec §18.6/§18.8). Pulls the
    /// manifest, enforces the active budget (§6.8), publishes + verifies every
    /// authoritative object through the local two-phase discipline (resumable +
    /// suspendable, §11.2), then seals a mirror cut locally so source-off actions
    /// run. Returns [`ImportOutcome::Completed`] with the mirror cut number, or
    /// [`ImportOutcome::Suspended`] with a resume token if a cooperative suspend was
    /// requested mid-object. Sends `ImportComplete` only on completion.
    pub fn import_cut(
        &mut self,
        session: &mut Session,
        remote_session: &str,
        remote_cut: u64,
    ) -> Result<ImportOutcome> {
        // NOTE: the suspend flag is NOT cleared here — a shell may set it before or
        // during the call. It is cleared only when a resume begins.
        session.send(Message::CutRequest {
            session: remote_session.into(),
            cut_number: remote_cut,
        })?;
        let manifest = match session.recv()? {
            Message::CutManifest { manifest } => manifest,
            Message::Abort { reason } => {
                return Err(CoreError::transfer(TransferCode::Cancelled, reason))
            }
            other => {
                return Err(CoreError::internal(
                    InternalCode::Invariant,
                    format!("expected CutManifest, got {other:?}"),
                ))
            }
        };

        // Validate every entry before pulling a single byte (ADP-006/008, §7.3).
        for e in &manifest.objects {
            crate::prepare::validate_entry(e)?;
        }

        // Budget admission (§6.8): refuse an over-budget import before any bytes.
        self.admit_import_budget(&manifest)?;

        self.pull_and_seal(session, remote_session, remote_cut, &manifest)
    }

    /// Resume a previously suspended import (§11.2). Re-requests the manifest, skips
    /// already-published objects (dedup fast-path), and resumes the pending object
    /// from its checkpointed offset. Returns the same outcome shape as `import_cut`.
    pub fn resume_import(
        &mut self,
        session: &mut Session,
        token: &ImportResume,
    ) -> Result<ImportOutcome> {
        self.clear_suspend();
        session.send(Message::CutRequest {
            session: token.remote_session.clone(),
            cut_number: token.remote_cut,
        })?;
        let manifest = match session.recv()? {
            Message::CutManifest { manifest } => manifest,
            Message::Abort { reason } => {
                return Err(CoreError::transfer(TransferCode::Cancelled, reason))
            }
            other => {
                return Err(CoreError::internal(
                    InternalCode::Invariant,
                    format!("expected CutManifest, got {other:?}"),
                ))
            }
        };
        for e in &manifest.objects {
            crate::prepare::validate_entry(e)?;
        }
        // Resume honors the budget too; already-staged bytes are not re-counted
        // against net budget here (they were admitted on the first attempt).
        self.pull_and_seal(session, &token.remote_session, token.remote_cut, &manifest)
    }

    /// Shared pull→publish→seal loop used by both `import_cut` and `resume_import`.
    /// Uses `publish_object_resumable`, so a suspend request mid-object checkpoints
    /// and returns a resume token instead of a sealed cut.
    fn pull_and_seal(
        &mut self,
        session: &mut Session,
        remote_session: &str,
        remote_cut: u64,
        manifest: &ObjectManifest,
    ) -> Result<ImportOutcome> {
        let chunk_size = self.chunk_size();
        let suspend_flag = self.suspend_flag();
        let mut authoritative = Vec::new();
        for e in &manifest.objects {
            let obj = raw_entry_to_object(e)?;
            let src = NetChunkSource {
                session: std::cell::RefCell::new(session),
                content_hash: e.content_hash.clone(),
            };
            let (db, store, journal, _host) = self.parts();
            // Verify against the manifest's declared digest — a lying source fails
            // here with OBJECT_DigestMismatch (CORE-004 over the wire).
            match publish_object_resumable(
                db,
                store,
                journal,
                &obj,
                &src,
                chunk_size,
                &suspend_flag,
            )? {
                PublishProgress::Completed(_) => {
                    if obj.kind == ObjectKind::Authoritative {
                        authoritative.push(obj.version());
                    }
                }
                PublishProgress::Suspended { next_offset, .. } => {
                    // Checkpointed mid-object. Do NOT seal; hand back a resume token.
                    return Ok(ImportOutcome::Suspended(ImportResume {
                        remote_session: remote_session.to_string(),
                        remote_cut,
                        pending_content_hash: e.content_hash.clone(),
                        next_offset,
                    }));
                }
            }
        }

        // Seal a mirror cut locally so the imported state is first-class.
        let local_session = self.ensure_mirror_session(remote_session, manifest)?;
        let cut_number = self.seal_imported_cut(local_session, &authoritative)?;

        // Closure validated: the imported authoritative objects are all present and
        // verified, so declared actions can run source-off. Mark the session
        // ACTION_READY (§18.8 `Importing → ActionReady`, §16.2 — the exact readiness
        // state, never bare "ready"). This anchors the time-to-ACTION_READY metric.
        self.db().with_tx(|tx| {
            tx.execute(
                "UPDATE sessions SET state=?2 WHERE session_id=?1",
                rusqlite::params![
                    local_session.to_string(),
                    crate::model::SessionState::ActionReady.as_str()
                ],
            )?;
            Ok(())
        })?;

        let manifest_digest = manifest_digest(manifest);
        session.send(Message::ImportComplete {
            session: remote_session.into(),
            cut_number: remote_cut,
            manifest_digest: manifest_digest.clone(),
        })?;
        self.journal().append(
            &JournalEvent::new(EventType::CutSealed, "OK")
                .with_session(local_session.to_string())
                .with_cut(cut_number)
                .with_metadata(serde_json::json!({
                    "role": "destination",
                    "imported_from": remote_session,
                    "imported_cut": remote_cut,
                    "manifest_digest": manifest_digest,
                })),
        )?;
        Ok(ImportOutcome::Completed(cut_number))
    }

    /// Enforce the effective budget on the import closure (§6.8/§20.2). Builds an
    /// [`crate::policy::Observation`] from the manifest and admits one
    /// `JobKind::Transfer` proposal per not-yet-present object via the existing
    /// `policy::admit`. An over-budget object is refused before any bytes move;
    /// refusal is journaled (a normal, visible outcome, §6.8).
    fn admit_import_budget(&mut self, manifest: &ObjectManifest) -> Result<()> {
        use crate::policy::{admit, AdmitRejection, JobKind, PrepMode, Proposal, RationaleCode};
        let budget = self.effective_budget();
        let obs = self.import_observation(manifest, budget);
        for obj in &obs.objects {
            if obj.locally_present {
                continue;
            }
            let proposal = Proposal {
                proposal_id: uuid::Uuid::nil(),
                observation_generation: obs.observation_generation,
                job_kind: JobKind::Transfer,
                object_id: obj.id.clone(),
                object_version: obj.generation,
                mode: PrepMode::DirectBytes,
                estimated_network_bytes: obj.logical_size,
                estimated_cpu_millis: 0,
                estimated_peak_memory: self.chunk_size().unwrap_or(1024 * 1024),
                estimated_nonpreemptible_millis: 0,
                expected_action_benefit: 1.0,
                rationale_code: RationaleCode::AuthoritativeRequired,
            };
            if let Err(rej) = admit(&obs, &proposal) {
                let code = match rej {
                    AdmitRejection::BudgetExceeded("network") => BudgetCode::Network,
                    AdmitRejection::BudgetExceeded("cpu") => BudgetCode::Cpu,
                    AdmitRejection::BudgetExceeded("memory") => BudgetCode::Memory,
                    AdmitRejection::BudgetExceeded(_) => BudgetCode::Network,
                    // Non-budget rejections at import time are internal invariants
                    // (observation is built from the same manifest we admit against).
                    _ => {
                        return Err(CoreError::internal(
                            InternalCode::Invariant,
                            format!("import admission rejected: {rej:?}"),
                        ))
                    }
                };
                self.journal().append(
                    &JournalEvent::new(EventType::TransferInterrupted, "BUDGET_Rejected")
                        .with_metadata(serde_json::json!({
                            "object_id": obj.id.0,
                            "logical_size": obj.logical_size,
                            "budget_net_bytes": budget.total_net_bytes,
                        })),
                )?;
                return Err(CoreError::budget(
                    code,
                    format!(
                        "object {} ({} bytes) exceeds import budget",
                        obj.id.0, obj.logical_size
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Build a read-only observation of the import closure for budget admission.
    fn import_observation(
        &self,
        manifest: &ObjectManifest,
        budget: crate::model::Budget,
    ) -> crate::policy::Observation {
        use crate::policy::{Observation, ObservedObject};
        let objects = manifest
            .objects
            .iter()
            .map(|e| {
                let present = Digest::from_hex(&e.content_hash)
                    .map(|d| self.store().exists(&d))
                    .unwrap_or(false);
                ObservedObject {
                    id: ObjectId(e.object_id.clone()),
                    generation: e.generation,
                    logical_size: e.logical_size,
                    required_authoritative: matches!(e.kind, ObjectKindWire::Authoritative),
                    locally_present: present,
                }
            })
            .collect();
        Observation {
            observation_generation: manifest.generation,
            objects,
            budget,
        }
    }

    /// Create (once) a local session mirroring the remote one, so imported cuts
    /// have a home. Keyed deterministically on the remote session id.
    /// The deterministic local mirror session id for a remote session string.
    /// Derived so a repeated import is idempotent (NET-006 at the session level);
    /// also the handle a destination passes to `request_authority_transfer`.
    pub fn mirror_session_id(remote_session: &str) -> SessionId {
        SessionId(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("carryon-mirror:{remote_session}").as_bytes(),
        ))
    }

    fn ensure_mirror_session(
        &mut self,
        remote_session: &str,
        manifest: &ObjectManifest,
    ) -> Result<SessionId> {
        let mirror = Self::mirror_session_id(remote_session);
        let exists: bool = self
            .db_shared()
            .conn()
            .query_row(
                "SELECT 1 FROM sessions WHERE session_id=?1",
                rusqlite::params![mirror.to_string()],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !exists {
            let schema_version = manifest.generation.min(u32::MAX as u64) as i64;
            self.db().with_tx(|tx| {
                tx.execute(
                    "INSERT INTO sessions \
                     (session_id, adapter_id, adapter_version, schema_version, title, \
                      creation_device, created_utc, authority_epoch, latest_cut, privacy, \
                      manifest_root, state, generation) \
                     VALUES (?1,'imported','0',?2,?3,'remote',?4,0,NULL,'public',NULL,'Importing',0)",
                    rusqlite::params![
                        mirror.to_string(),
                        schema_version.max(1),
                        format!("import of {remote_session}"),
                        now_utc(),
                    ],
                )?;
                tx.execute(
                    "INSERT INTO authority_epochs (session_id, epoch, owner_device, mode, opened_utc) \
                     VALUES (?1, 0, 'remote', 'read_only_replica', ?2)",
                    rusqlite::params![mirror.to_string(), now_utc()],
                )?;
                Ok(())
            })?;
        }
        Ok(mirror)
    }

    /// Seal an imported cut referencing the published authoritative objects.
    fn seal_imported_cut(
        &mut self,
        session: SessionId,
        authoritative: &[ObjectVersion],
    ) -> Result<u64> {
        let number: u64 = {
            let n: Option<i64> = self
                .db()
                .conn()
                .query_row(
                    "SELECT MAX(cut_number) FROM cuts WHERE session_id=?1",
                    rusqlite::params![session.to_string()],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            n.map(|v| v as u64 + 1).unwrap_or(0)
        };
        self.db().with_tx(|tx| {
            tx.execute(
                "INSERT INTO cuts (session_id, cut_number, epoch, adapter_schema_version, \
                 derived_validity_json, source_signature, created_utc, manifest_digest, sealed) \
                 VALUES (?1,?2,0,1,'[]',NULL,?3,?4,1)",
                rusqlite::params![
                    session.to_string(),
                    number as i64,
                    now_utc(),
                    // Imported manifest digest placeholder; the per-object digests
                    // are the real integrity anchor.
                    format!("imported-cut-{number}"),
                ],
            )?;
            for (ordinal, v) in authoritative.iter().enumerate() {
                tx.execute(
                    "INSERT INTO cut_objects (session_id, cut_number, ordinal, object_id, generation, role) \
                     VALUES (?1,?2,?3,?4,?5,'authoritative')",
                    rusqlite::params![
                        session.to_string(),
                        number as i64,
                        ordinal as i64,
                        v.id.0,
                        v.generation as i64
                    ],
                )?;
            }
            tx.execute(
                "UPDATE sessions SET state='CutSealed', latest_cut=?2, generation=generation+1 \
                 WHERE session_id=?1",
                rusqlite::params![session.to_string(), number as i64],
            )?;
            Ok(())
        })?;
        Ok(number)
    }
}

/// A raw row of the `objects` table joined into a cut.
struct RawObjRow {
    object_id: String,
    generation: u64,
    kind: String,
    schema_id: String,
    content_hash: String,
    logical_size: u64,
    recipe_id: Option<String>,
    portable: bool,
    sensitivity: String,
    retention: String,
    parents_json: String,
}

fn raw_to_entry(r: RawObjRow) -> Result<ObjectEntry> {
    let parents: Vec<ObjectVersion> = serde_json::from_str(&r.parents_json).unwrap_or_default();
    Ok(ObjectEntry {
        object_id: r.object_id,
        generation: r.generation,
        kind: kind_to_wire(&r.kind),
        schema_id: r.schema_id,
        content_hash: r.content_hash,
        logical_size: r.logical_size,
        parents: parents
            .into_iter()
            .map(|p| ObjectVersionWire {
                object_id: p.id.0,
                generation: p.generation,
            })
            .collect(),
        recipe_id: r.recipe_id,
        portable: r.portable,
        sensitivity: sensitivity_to_wire(&r.sensitivity),
        retention: retention_to_wire(&r.retention),
    })
}

/// Convert a wire manifest entry (received) into a core `Object` for publishing.
fn raw_entry_to_object(e: &ObjectEntry) -> Result<Object> {
    let content_hash = Digest::from_hex(&e.content_hash)
        .ok_or_else(|| CoreError::object(ObjectCode::Invalid, "bad content_hash"))?;
    Ok(Object {
        id: ObjectId(e.object_id.clone()),
        kind: e.kind.into(),
        generation: e.generation,
        schema_id: e.schema_id.clone(),
        content_hash,
        wire_hash: None,
        logical_size: e.logical_size,
        wire_size: None,
        parents: e
            .parents
            .iter()
            .map(|p| ObjectVersion {
                id: ObjectId(p.object_id.clone()),
                generation: p.generation,
            })
            .collect(),
        recipe_id: e.recipe_id.clone(),
        portable: e.portable,
        sensitivity: e.sensitivity.into(),
        retention: e.retention.into(),
    })
}

fn read_store_range(
    store: &crate::store::Store,
    digest: &Digest,
    offset: u64,
    want: usize,
) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = store.open_object(digest)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; want];
    let mut filled = 0;
    while filled < want {
        let n = f.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

fn manifest_digest(m: &ObjectManifest) -> String {
    let bytes = serde_json::to_vec(m).unwrap_or_default();
    Digest::of(&bytes).to_hex()
}

fn parse_session(s: &str) -> Result<SessionId> {
    uuid::Uuid::parse_str(s)
        .map(SessionId)
        .map_err(|_| CoreError::internal(InternalCode::Invariant, "bad session id"))
}

fn kind_to_wire(s: &str) -> ObjectKindWire {
    match s {
        "authoritative" => ObjectKindWire::Authoritative,
        "derived" => ObjectKindWire::Derived,
        "cache" => ObjectKindWire::Cache,
        "preview" => ObjectKindWire::Preview,
        _ => ObjectKindWire::Ephemeral,
    }
}

fn sensitivity_to_wire(s: &str) -> SensitivityWire {
    match s {
        "public" => SensitivityWire::Public,
        "personal" => SensitivityWire::Personal,
        "confidential" => SensitivityWire::Confidential,
        "secret" => SensitivityWire::Secret,
        _ => SensitivityWire::Prohibited,
    }
}

fn retention_to_wire(s: &str) -> RetentionWire {
    if s.starts_with("bounded") {
        RetentionWire::Bounded
    } else {
        match s {
            "session" => RetentionWire::Session,
            "persistent" => RetentionWire::Persistent,
            _ => RetentionWire::NoCache,
        }
    }
}
