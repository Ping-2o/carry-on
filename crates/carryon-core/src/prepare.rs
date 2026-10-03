//! Snapshot → cut → publish orchestration and action execution (spec §10.4,
//! §6.6, §7). The trickiest invariant lives here: a cut becomes visible only
//! after all its authoritative objects are published+verified and the seal is
//! committed, with the journal line written only after the commit (CORE-004/005).
//!
//! Phase 1 reads each object's bytes fully through one crash-contained
//! `host.call` (objects are ≤ 16 MiB, §host::CORE_MAX_OBJECT_BYTES), then
//! publishes from an in-memory slice. This keeps the two-phase digest/verify/
//! atomic-publish discipline without a borrow conflict between the host (which
//! needs `&mut`) and the per-chunk `ChunkSource::read` (`&self`).

use crate::core::Core;
use crate::db::migrations::now_utc;
use crate::error::{ActionCode, CoreError, InternalCode, ObjectCode, Result, SchemaCode};
use crate::ids::{CutId, Digest, ObjectId, ObjectVersion, SessionId};
use crate::journal::{EventType, JournalEvent};
use crate::model::object::{Object, ObjectKind, Sensitivity};
use crate::model::{
    ActionAvailability, ActionRequest, ActionResult, Cut, OracleOutcome, SessionState,
};
use crate::store::publish::publish_object;
use crate::store::ChunkSource;
use carryon_adapter_api::{ActionRequestWire, CutRef, ObjectEntry, SnapshotToken};

/// An in-memory byte source for a fully-read object (Phase 1).
struct SliceSource {
    bytes: Vec<u8>,
}

impl ChunkSource for SliceSource {
    fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>> {
        let start = (offset as usize).min(self.bytes.len());
        let end = (start + length as usize).min(self.bytes.len());
        Ok(self.bytes[start..end].to_vec())
    }
}

impl Core {
    /// Seal a cut: snapshot the adapter's session, validate the manifest, publish
    /// every object, then seal in one transaction. Read-only (`CutSealed`).
    pub fn create_cut(&mut self, session: SessionId) -> Result<CutId> {
        let sess = self
            .get_session(session)
            .ok_or_else(|| CoreError::internal(InternalCode::Invariant, "no such session"))?;
        let adapter_id = sess.adapter_id.clone();

        self.set_session_state(session, SessionState::Preparing)?;

        // 1. Snapshot: current generation → begin → describe.
        let session_key = session.to_string();
        let generation = self.host().call(&adapter_id, |a| {
            a.current_generation(&session_key).map_err(CoreError::from)
        })?;
        let token = self.host().call(&adapter_id, |a| {
            a.begin_snapshot(&session_key, generation)
                .map_err(CoreError::from)
        })?;
        let manifest = self.host().call(&adapter_id, |a| {
            a.describe_snapshot(&token).map_err(CoreError::from)
        })?;

        // 2. Validate every object before any persistence (ADP-006/008, §7.3).
        for e in &manifest.objects {
            validate_entry(e)?;
        }

        // 3. Publish each object via two-phase publish; track authoritative ones.
        let chunk_size = self.chunk_size();
        let mut authoritative = Vec::new();
        for e in &manifest.objects {
            let obj = entry_to_object(e)?;
            let bytes = self.read_object_full(&adapter_id, &token, &e.object_id, e.logical_size)?;
            // Core hashes the adapter-served bytes and verifies against the
            // declared content_hash inside publish_object (OBJECT_DigestMismatch
            // on a lying manifest).
            let source = SliceSource { bytes };
            let (db, store, journal, _host) = self.parts();
            publish_object(db, store, journal, &obj, &source, chunk_size)?;
            if obj.kind == ObjectKind::Authoritative {
                authoritative.push(obj.version());
            }
        }

        let _ = self.host().call(&adapter_id, |a| {
            a.finish_snapshot(token.clone()).map_err(CoreError::from)
        });

        // 4. Compute + 5. seal the cut in one transaction.
        let number = self.next_cut_number(session)?;
        let cut = Cut {
            session,
            number,
            epoch: sess.authority_epoch,
            authoritative_versions: authoritative.clone(),
            adapter_schema_version: sess.schema_version,
            recipe_versions: vec![],
            derived_validity: vec![],
            source_signature: None,
            created_utc: now_utc(),
            manifest_digest: Digest([0; 32]),
        };
        let digest = cut.compute_digest();

        self.db().with_tx(|tx| {
            tx.execute(
                "INSERT INTO cuts (session_id, cut_number, epoch, adapter_schema_version, \
                 derived_validity_json, source_signature, created_utc, manifest_digest, sealed) \
                 VALUES (?1,?2,?3,?4,'[]',NULL,?5,?6,1)",
                rusqlite::params![
                    session.to_string(),
                    number as i64,
                    cut.epoch.0 as i64,
                    cut.adapter_schema_version as i64,
                    now_utc(),
                    digest.to_hex(),
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
                "UPDATE sessions SET generation=generation+1, latest_cut=?2, state=?3, manifest_root=?4 \
                 WHERE session_id=?1",
                rusqlite::params![
                    session.to_string(),
                    number as i64,
                    SessionState::CutSealed.as_str(),
                    digest.to_hex(),
                ],
            )?;
            Ok(())
        })?;

        // 6. Journal CUT_SEALED only after the commit.
        self.journal().append(
            &JournalEvent::new(EventType::CutSealed, "OK")
                .with_session(session.to_string())
                .with_cut(number)
                .with_metadata(serde_json::json!({
                    "manifest_digest": digest.to_hex(),
                    "generation": generation,
                })),
        )?;

        Ok(CutId { session, number })
    }

    /// Which actions can run now at a cut (all authoritative objects present).
    pub fn list_available_actions(
        &self,
        cut: CutId,
        classes: &[String],
    ) -> Vec<ActionAvailability> {
        let missing = self.missing_authoritative(cut);
        classes
            .iter()
            .map(|c| ActionAvailability {
                class: c.clone(),
                ready: missing.is_empty(),
                missing: missing.clone(),
            })
            .collect()
    }

    /// The object ids sealed as authoritative into a cut (ordered). Navigation and
    /// other optional/ephemeral objects are published but not sealed, so they do not
    /// appear here — this is the "transferred (authoritative) vs optional" split used
    /// by continuation metrics and the benchmark harness.
    pub fn cut_authoritative_objects(&self, cut: CutId) -> Vec<String> {
        let conn = self.db_shared().conn();
        let mut stmt = match conn.prepare(
            "SELECT object_id FROM cut_objects \
             WHERE session_id=?1 AND cut_number=?2 AND role='authoritative' \
             ORDER BY object_id",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(
            rusqlite::params![cut.session.to_string(), cut.number as i64],
            |r| r.get::<_, String>(0),
        )
        .and_then(|it| it.collect::<std::result::Result<Vec<_>, _>>())
        .unwrap_or_default()
    }

    /// Execute a declared action at a sealed cut (read-only; no authority change).
    /// The adapter is the session's recorded one. For an imported **mirror** session
    /// (recorded adapter id `imported`), use [`Core::execute_action_as`] to name the
    /// registered adapter that continues it.
    pub fn execute_action(&mut self, cut: CutId, request: ActionRequest) -> Result<ActionResult> {
        let sess = self
            .get_session(cut.session)
            .ok_or_else(|| CoreError::internal(InternalCode::Invariant, "no such session"))?;
        let adapter_id = sess.adapter_id.clone();
        self.execute_action_as(cut, request, &adapter_id)
    }

    /// Execute a declared action at a sealed cut, driven by an explicitly named
    /// registered adapter. Needed for source-off continuation of an imported mirror
    /// session, whose recorded adapter id is the placeholder `imported` rather than a
    /// registered adapter.
    pub fn execute_action_as(
        &mut self,
        cut: CutId,
        request: ActionRequest,
        adapter_id: &str,
    ) -> Result<ActionResult> {
        let adapter_id = adapter_id.to_string();

        if !self.cut_is_sealed(cut)? {
            return Err(CoreError::object(ObjectCode::Stale, "cut is not sealed"));
        }

        let cut_ref = CutRef {
            session: cut.session.to_string(),
            cut_number: cut.number,
        };
        let wire = ActionRequestWire {
            class: request.class.clone(),
            params: request.params.clone(),
        };

        // Resolve dependencies; verify prerequisites are present (source-off).
        let plan = self.host().call(&adapter_id, |a| {
            a.resolve_action(&cut_ref, &wire).map_err(CoreError::from)
        })?;
        for pre in &plan.prerequisites {
            let digest = self.object_digest(&pre.object_id, pre.generation)?;
            if !self.store().exists(&digest) {
                return Err(CoreError::action(
                    ActionCode::DependencyFailure,
                    format!(
                        "prerequisite {} gen {} not present",
                        pre.object_id, pre.generation
                    ),
                ));
            }
        }

        let result = self.host().call(&adapter_id, |a| {
            a.execute_action(&cut_ref, &wire).map_err(CoreError::from)
        })?;

        let outcome = OracleOutcome {
            checked: result.oracle.checked,
            agreed: result.oracle.agreed,
            output_hash: Digest::from_hex(&result.oracle.output_hash).unwrap_or(Digest([0; 32])),
            detail: result.oracle.detail.clone(),
        };
        if outcome.checked && !outcome.agreed {
            return Err(CoreError::action(ActionCode::OracleFailure, outcome.detail));
        }

        let action_id = uuid::Uuid::new_v4().to_string();
        let params_json = serde_json::to_string(&request.params)?;
        let output_json = serde_json::to_string(&result.output)?;
        self.db().with_tx(|tx| {
            tx.execute(
                "INSERT INTO actions (action_id, session_id, cut_number, class, params_json, mutates, \
                 result_json, oracle_outcome, created_utc) VALUES (?1,?2,?3,?4,?5,0,?6,?7,?8)",
                rusqlite::params![
                    action_id,
                    cut.session.to_string(),
                    cut.number as i64,
                    request.class,
                    params_json,
                    output_json,
                    result.oracle.detail,
                    now_utc(),
                ],
            )?;
            Ok(())
        })?;
        self.journal().append(
            &JournalEvent::new(EventType::ActionExecuted, "OK")
                .with_session(cut.session.to_string())
                .with_cut(cut.number),
        )?;

        Ok(ActionResult {
            output: result.output,
            output_hash: Digest::from_hex(&result.output_hash).unwrap_or(Digest([0; 32])),
            oracle: outcome,
        })
    }

    // --- helpers ---

    /// Read an object's full bytes through one crash-contained host call.
    fn read_object_full(
        &mut self,
        adapter_id: &str,
        token: &SnapshotToken,
        object_id: &str,
        logical_size: u64,
    ) -> Result<Vec<u8>> {
        let token = token.clone();
        let object_id = object_id.to_string();
        self.host().call(adapter_id, move |a| {
            let mut out = Vec::with_capacity(logical_size as usize);
            let mut offset = 0u64;
            loop {
                let chunk = a
                    .read_object(&token, &object_id, offset, 1024 * 1024)
                    .map_err(CoreError::from)?;
                if chunk.is_empty() {
                    break;
                }
                offset += chunk.len() as u64;
                out.extend_from_slice(&chunk);
                if offset >= logical_size {
                    break;
                }
            }
            Ok(out)
        })
    }

    fn set_session_state(&mut self, session: SessionId, state: SessionState) -> Result<()> {
        self.db().with_tx(|tx| {
            tx.execute(
                "UPDATE sessions SET state=?2 WHERE session_id=?1",
                rusqlite::params![session.to_string(), state.as_str()],
            )?;
            Ok(())
        })
    }

    fn next_cut_number(&mut self, session: SessionId) -> Result<u64> {
        let n: Option<i64> = self
            .db()
            .conn()
            .query_row(
                "SELECT MAX(cut_number) FROM cuts WHERE session_id=?1",
                rusqlite::params![session.to_string()],
                |r| r.get(0),
            )
            .unwrap_or(None);
        Ok(n.map(|v| v as u64 + 1).unwrap_or(0))
    }

    fn cut_is_sealed(&mut self, cut: CutId) -> Result<bool> {
        let sealed: Option<i64> = self
            .db()
            .conn()
            .query_row(
                "SELECT sealed FROM cuts WHERE session_id=?1 AND cut_number=?2",
                rusqlite::params![cut.session.to_string(), cut.number as i64],
                |r| r.get(0),
            )
            .ok();
        Ok(sealed == Some(1))
    }

    fn object_digest(&mut self, object_id: &str, generation: u64) -> Result<Digest> {
        let hex: String = self
            .db()
            .conn()
            .query_row(
                "SELECT content_hash FROM objects WHERE object_id=?1 AND generation=?2",
                rusqlite::params![object_id, generation as i64],
                |r| r.get(0),
            )
            .map_err(|_| {
                CoreError::object(
                    ObjectCode::Missing,
                    format!("object {object_id} gen {generation}"),
                )
            })?;
        Digest::from_hex(&hex)
            .ok_or_else(|| CoreError::object(ObjectCode::Invalid, "bad stored digest"))
    }

    fn missing_authoritative(&self, cut: CutId) -> Vec<ObjectVersion> {
        let conn = self.db_shared().conn();
        let mut stmt = match conn.prepare(
            "SELECT o.object_id, o.generation, o.content_hash FROM cut_objects c \
             JOIN objects o ON o.object_id=c.object_id AND o.generation=c.generation \
             WHERE c.session_id=?1 AND c.cut_number=?2 AND c.role='authoritative'",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt
            .query_map(
                rusqlite::params![cut.session.to_string(), cut.number as i64],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .and_then(|it| it.collect::<std::result::Result<Vec<_>, _>>())
            .unwrap_or_default();
        rows.into_iter()
            .filter(|(_, _, hex)| {
                !Digest::from_hex(hex)
                    .map(|d| self.store().exists(&d))
                    .unwrap_or(false)
            })
            .map(|(id, g, _)| ObjectVersion {
                id: ObjectId(id),
                generation: g as u64,
            })
            .collect()
    }
}

/// Validate a declared manifest entry (schema/caps/kind/secret exclusion). Shared
/// with the cross-device import path so a remote manifest is validated by the same
/// rules as a local snapshot (ADP-006/008, §7.3).
pub(crate) fn validate_entry(e: &ObjectEntry) -> Result<()> {
    if e.schema_id.is_empty() {
        return Err(CoreError::schema(SchemaCode::Invalid, "empty schema_id"));
    }
    if e.logical_size > crate::host::CORE_MAX_OBJECT_BYTES {
        return Err(CoreError::object(
            ObjectCode::Oversized,
            format!("object {} exceeds core cap", e.object_id),
        ));
    }
    let sensitivity: Sensitivity = e.sensitivity.into();
    if sensitivity.is_excluded_by_default() {
        return Err(CoreError::object(
            ObjectCode::SecretExcluded,
            format!(
                "object {} is {} and excluded by default",
                e.object_id,
                sensitivity.as_str()
            ),
        ));
    }
    if Digest::from_hex(&e.content_hash).is_none() {
        return Err(CoreError::object(
            ObjectCode::Invalid,
            "content_hash is not 64 hex chars",
        ));
    }
    Ok(())
}

fn entry_to_object(e: &ObjectEntry) -> Result<Object> {
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
