//! Evidence bundle assembly and verification (spec §23.3).
//!
//! A bundle is a single JSON document plus a clear statement of what it does and
//! does not prove. `verify_evidence` only parses and hash-checks — it NEVER
//! executes any bundle content (EVD-005). Failures remain in the record
//! (EVD-003); no pairing secrets are included (EVD-006, vacuous in Phase 1).

use crate::core::Core;
use crate::db::migrations::now_utc;
use crate::error::{CoreError, InternalCode, Result};
use crate::ids::{Digest, SessionId};
use serde::{Deserialize, Serialize};

/// A self-describing evidence bundle (§23.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub bundle_version: u32,
    pub session_id: String,
    pub created_utc: String,
    pub verifier_version: String,
    /// Build/config identifiers.
    pub build: BuildInfo,
    /// Session + cut + object metadata (no payloads).
    pub session_json: serde_json::Value,
    /// Journal events for the session.
    pub journal: Vec<serde_json::Value>,
    /// Action results + oracle outcomes.
    pub actions: Vec<serde_json::Value>,
    /// Aggregate metrics (§23.2).
    pub metrics: serde_json::Value,
    /// Per-section integrity hashes.
    pub section_hashes: Vec<(String, String)>,
    /// The explicit what-it-proves / what-it-does-not-prove statement (§23.3).
    pub disclosure: String,
}

/// Build/config identifiers for reproducibility (§23.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub crate_version: String,
    pub schema_version: u32,
    pub phase: String,
}

/// Report from verifying a bundle on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub ok: bool,
    pub message: String,
}

const DISCLOSURE: &str = "This bundle records a LOCAL Phase-1 run of the Carry-On engine. \
It proves that the named session's cuts, objects, and actions were produced and validated on THIS \
machine under the stated build. It does NOT prove any cross-device transfer, network transport, \
performance claim, or that arbitrary applications satisfy the block-sum model (spec §2, §30).";

impl Core {
    /// Assemble an evidence bundle for a session (§23.3).
    pub fn export_evidence(&mut self, session: SessionId) -> Result<EvidenceBundle> {
        self.export_evidence_with(session, serde_json::Value::Null)
    }

    /// Assemble an evidence bundle, merging caller-supplied continuation metrics
    /// (measured timings, transferred bytes, transferred-vs-optional objects, the
    /// oracle result, `endpoint_changed_symbols`, …) into the `metrics` section.
    /// The driver measures these around the handoff and injects them here, so they
    /// are covered by the bundle's section hashes like every other metric.
    pub fn export_evidence_with(
        &mut self,
        session: SessionId,
        extra_metrics: serde_json::Value,
    ) -> Result<EvidenceBundle> {
        let session_json = self
            .get_session(session)
            .map(|s| serde_json::to_value(&s))
            .transpose()?
            .unwrap_or(serde_json::Value::Null);

        let key = session.to_string();
        let journal = self.journal_rows(&key)?;
        let actions = self.action_rows(&key)?;
        let mut metrics = self.metrics_for(&key)?;
        // Merge injected continuation metrics (never overwriting the base counts).
        if let (Some(base), Some(extra)) = (metrics.as_object_mut(), extra_metrics.as_object()) {
            for (k, v) in extra {
                base.insert(k.clone(), v.clone());
            }
        }

        let mut bundle = EvidenceBundle {
            bundle_version: 1,
            session_id: key.clone(),
            created_utc: now_utc(),
            verifier_version: "carryon-core/0.1".into(),
            build: BuildInfo {
                crate_version: env!("CARGO_PKG_VERSION").into(),
                schema_version: 1,
                phase: "phase-1-local".into(),
            },
            session_json,
            journal,
            actions,
            metrics,
            section_hashes: Vec::new(),
            disclosure: DISCLOSURE.into(),
        };
        bundle.section_hashes = bundle.compute_section_hashes()?;

        // Record the bundle in the DB index + journal.
        let bundle_json = serde_json::to_string(&bundle)?;
        let evidence_id = uuid::Uuid::new_v4().to_string();
        self.db().with_tx(|tx| {
            tx.execute(
                "INSERT INTO evidence_bundles (evidence_id, session_id, created_utc, manifest_json, \
                 bundle_path, verifier_version) VALUES (?1,?2,?3,?4,'inline',?5)",
                rusqlite::params![evidence_id, key, now_utc(), bundle_json, bundle.verifier_version],
            )?;
            Ok(())
        })?;
        self.journal().append(
            &crate::journal::JournalEvent::new(crate::journal::EventType::EvidenceExported, "OK")
                .with_session(key),
        )?;

        Ok(bundle)
    }

    fn journal_rows(&self, session: &str) -> Result<Vec<serde_json::Value>> {
        // The journal file is the source of truth for the hash-chained event log
        // (§23.1). Read it back and keep events for this session (or global events
        // with no session binding, like RECOVERY_REPORT).
        let events = crate::journal::Journal::read_all(self.journal_path())?;
        let rows = events
            .into_iter()
            .filter(|e| e.session_id.as_deref() == Some(session))
            .map(|e| {
                serde_json::json!({
                    "event_type": e.event_type.as_str(),
                    "result_code": e.result_code,
                    "utc": e.utc,
                    "metadata": e.metadata,
                })
            })
            .collect();
        Ok(rows)
    }

    fn action_rows(&self, session: &str) -> Result<Vec<serde_json::Value>> {
        let conn = self.db_shared().conn();
        let mut stmt = conn.prepare(
            "SELECT class, result_json, oracle_outcome, created_utc FROM actions \
             WHERE session_id=?1 ORDER BY created_utc",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![session], |r| {
                Ok(serde_json::json!({
                    "class": r.get::<_, String>(0)?,
                    "result": r.get::<_, Option<String>>(1)?,
                    "oracle": r.get::<_, Option<String>>(2)?,
                    "utc": r.get::<_, String>(3)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn metrics_for(&self, session: &str) -> Result<serde_json::Value> {
        let conn = self.db_shared().conn();
        let cuts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cuts WHERE session_id=?1 AND sealed=1",
                rusqlite::params![session],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let objects: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT object_id) FROM cut_objects WHERE session_id=?1",
                rusqlite::params![session],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let actions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM actions WHERE session_id=?1",
                rusqlite::params![session],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(serde_json::json!({
            "sealed_cuts": cuts,
            "distinct_objects": objects,
            "actions": actions,
        }))
    }
}

impl EvidenceBundle {
    /// Per-section integrity hashes over the canonical JSON of each section.
    fn compute_section_hashes(&self) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for (name, value) in [
            ("session", &self.session_json),
            ("journal", &serde_json::to_value(&self.journal)?),
            ("actions", &serde_json::to_value(&self.actions)?),
            ("metrics", &self.metrics),
        ] {
            let bytes = serde_json::to_vec(value)?;
            out.push((name.to_string(), Digest::of(&bytes).to_hex()));
        }
        Ok(out)
    }

    /// Verify a bundle's internal integrity: re-hash each section and compare to
    /// `section_hashes`. NEVER executes bundle content (EVD-005).
    pub fn verify(&self) -> VerifyReport {
        let recomputed = match self.compute_section_hashes() {
            Ok(h) => h,
            Err(e) => {
                return VerifyReport {
                    ok: false,
                    message: e.to_string(),
                }
            }
        };
        if recomputed == self.section_hashes {
            VerifyReport {
                ok: true,
                message: "section hashes match".into(),
            }
        } else {
            VerifyReport {
                ok: false,
                message: "section hash mismatch".into(),
            }
        }
    }
}

impl Core {
    /// Verify an evidence bundle read from a JSON file. Parses + hash-checks only;
    /// never executes any bundle content (EVD-005).
    pub fn verify_evidence(&self, bundle_path: &std::path::Path) -> Result<VerifyReport> {
        let text = std::fs::read_to_string(bundle_path)?;
        let bundle: EvidenceBundle = serde_json::from_str(&text)
            .map_err(|e| CoreError::internal(InternalCode::Serialization, e.to_string()))?;
        Ok(bundle.verify())
    }
}
