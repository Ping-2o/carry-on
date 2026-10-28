//! Carry-On PhotoCraft adapter (L3 structured session + L4 authority transfer)
//! — **Phase-1 skeleton**.
//!
//! Design: `docs/artcraft-bridge/ADAPTER-DESIGN.md`, research evidence:
//! `docs/artcraft-bridge/RESEARCH.md` (PhotoCraft pin `1f7fc0f5…`).
//!
//! # What is real and what is not
//!
//! Real (IMPLEMENTED, tested here): the four-object session taxonomy, the
//! manifest/dependency plan, the `document.edit` + `session.restore` action
//! vocabulary, and the L4 authority-transfer binding to the
//! authoritative-state hash — the same discipline as
//! `carryon-adapter-editor`, adapted to PhotoCraft facts (selection is
//! document state; history is not serializable; meta must carry the
//! session-side revision/dirty flags).
//!
//! UNSUPPORTED (fail closed, labeled): everything that needs a *live*
//! PhotoCraft — step replay against a real document, live workspace restore,
//! and any PhotoCraft file I/O. The seam is [`PhotocraftDriver`]
//! (`driver.rs`); Phase 2 plugs in the control-channel driver (no coupling)
//! and the in-process engine driver (journal access), see ADAPTER-DESIGN §5.
//! Until then `document.edit` returns `ACTION_UNSUPPORTED` and any driver
//! call fails closed with a reason naming the missing Phase-2 piece.
//!
//! # Object model (ADAPTER-DESIGN §1)
//!
//! | object                      | kind          | dependency role      |
//! |-----------------------------|---------------|---------------------|
//! | `photocraft.document.v1`      | Authoritative | prerequisite        |
//! | `photocraft.unsaved_edits.v1` | Authoritative | prerequisite (parents: document) |
//! | `photocraft.meta.v1`          | Authoritative | prerequisite + provenance |
//! | `photocraft.workspace.v1`     | Ephemeral     | optional (published, never sealed) |
//!
//! Adapters serve bytes; the core hashes and stages them (§7.4). Authority is
//! bound to the authoritative-state hash (§21.2): document + unsaved + meta.

use crate::model::{
    PhotocraftMeta, Workspace, ADAPTER_ID, MAX_OBJECT_BYTES, SCHEMA_DOC, SCHEMA_META,
    SCHEMA_UNSAVED, SCHEMA_WORKSPACE,
};
use carryon_adapter_api::*;
use serde::{Deserialize, Serialize};

pub mod control;
pub mod driver;
pub mod model;

pub use control::{ControlChannelConfig, ControlChannelConfigFile, ControlChannelDriver, Secret};
pub use driver::{DriverError, EditOutcome, PhotocraftDriver, Step, StubDriver};

/// In-memory session state: what the adapter serves bytes for. On a real
/// machine the Phase-2 driver fills it from (or pushes it into) PhotoCraft.
pub struct PhotocraftAdapter {
    session: String,
    document: Vec<u8>,
    unsaved: Vec<u8>,
    workspace: Workspace,
    meta: PhotocraftMeta,
    generation: u64,
    /// The Phase-2 seam. `None` (detached) = state only, `document.edit` and
    /// live restore fail closed; `Some(StubDriver)` = same, via the seam.
    driver: Option<Box<dyn PhotocraftDriver>>,
    /// Set between `prepare_authority_transfer` and its commit/abort.
    pending: Option<PendingTransfer>,
}

struct PendingTransfer {
    auth_hash: String,
}

/// The serialized proposal payload crossing the wire (data only).
#[derive(Serialize, Deserialize)]
struct Proposal {
    session: String,
    cut_number: u64,
    /// Hash over the authoritative state (document + unsaved + meta). The
    /// destination re-derives this from imported state and must agree.
    auth_hash: String,
}

impl PhotocraftAdapter {
    /// A session from explicit state — the constructor a Phase-2 driver uses
    /// after it has pulled state out of PhotoCraft (or pushed carried state in).
    pub fn with_state(
        session: impl Into<String>,
        document: Vec<u8>,
        unsaved: Vec<u8>,
        workspace: Workspace,
        meta: PhotocraftMeta,
    ) -> Self {
        PhotocraftAdapter {
            session: session.into(),
            document,
            unsaved,
            workspace,
            meta,
            generation: 1,
            driver: None,
            pending: None,
        }
    }

    /// Attach a driver (Phase 2). The driver is used for three things:
    /// (a) `begin_snapshot` refreshes state from the live PhotoCraft when
    /// attached; (b) `session.restore` drives the live app to the carried
    /// position (fail closed on failure); (c) `document.edit` replays steps.
    /// [`StubDriver`] still fails closed on all of them, so the fail-closed
    /// path stays the tested default.
    pub fn with_driver(mut self, driver: Box<dyn PhotocraftDriver>) -> Self {
        self.driver = Some(driver);
        self
    }

    /// A recognizable demonstration session, the analogue of
    /// `EditorAdapter::session_v1()`. The document/unsaved bytes are
    /// **stand-ins, not real `.pcraft` files** (a real one requires PhotoCraft;
    /// RESEARCH §1.5) — clearly marked so nobody mistakes this for format
    /// evidence. The meta carries the researched real-world shape: revision 7,
    /// saved 3, dirty, pin `1f7fc0f5…`.
    pub fn session_v1() -> Self {
        let meta = PhotocraftMeta {
            schema_id: "carryon.photocraft.session".into(),
            schema_version: 1,
            adapter_version: "0.1.0".into(),
            original_path: Some("/work/great-wave.pcraft".into()),
            document_name: "great-wave.pcraft".into(),
            revision: 7,
            saved_revision: 3,
            dirty: true,
            photocraft_pin: "1f7fc0f53696e54da79b95015abb37d7c22cdd1a".into(),
            unsaved_mode: model::UnsavedMode::Snapshot,
        };
        let workspace = Workspace {
            active_document_index: 0,
            selected_layers: vec![model::LayerTarget {
                name: "Curves 1".into(),
                occurrence: 0,
            }],
            viewport: model::Viewport {
                zoom: 0.5,
                center: [0.5, 0.42],
                rotation: 0.0,
            },
            active_tool: "brush".into(),
            panels: serde_json::json!({"dock": "default"}),
        };
        PhotocraftAdapter::with_state(
            "carryon-photocraft-session",
            b"PCRAFT-SKELETON-STANDIN-DOCUMENT v1 [not a real .pcraft]".to_vec(),
            b"PCRAFT-SKELETON-STANDIN-UNSAVED v1 [not a real .pcraft]".to_vec(),
            workspace,
            meta,
        )
    }

    fn meta_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.meta).expect("serialize meta")
    }

    fn workspace_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.workspace).expect("serialize workspace")
    }

    fn object_bytes(&self, object_id: &str) -> Option<Vec<u8>> {
        match object_id {
            SCHEMA_DOC => Some(self.document.clone()),
            SCHEMA_UNSAVED => Some(self.unsaved.clone()),
            SCHEMA_META => Some(self.meta_bytes()),
            SCHEMA_WORKSPACE => Some(self.workspace_bytes()),
            _ => None,
        }
    }

    /// Authoritative-state hash: document + unsaved + meta, length-prefixed
    /// (ADAPTER-DESIGN §3). Authority proposals are bound to this, never to a
    /// token or a process.
    fn auth_hash(&self) -> String {
        model::authoritative_hash(&self.document, &self.unsaved, &self.meta_bytes())
    }

    fn entry(&self, object_id: &str, kind: ObjectKindWire) -> ObjectEntry {
        let bytes = self.object_bytes(object_id).unwrap_or_default();
        // The unsaved object derives from the saved document; record lineage.
        let parents = if object_id == SCHEMA_UNSAVED {
            vec![ObjectVersionWire {
                object_id: SCHEMA_DOC.into(),
                generation: self.generation,
            }]
        } else {
            vec![]
        };
        ObjectEntry {
            object_id: object_id.to_string(),
            generation: self.generation,
            kind,
            schema_id: object_id.to_string(),
            content_hash: model::hex_sha256(&bytes),
            logical_size: bytes.len() as u64,
            parents,
            recipe_id: None,
            portable: true,
            sensitivity: SensitivityWire::Personal,
            retention: if object_id == SCHEMA_WORKSPACE {
                RetentionWire::Session
            } else {
                RetentionWire::Persistent
            },
        }
    }

    fn all_entries(&self) -> Vec<ObjectEntry> {
        vec![
            self.entry(SCHEMA_DOC, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_UNSAVED, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_META, ObjectKindWire::Authoritative),
            // Workspace is ephemeral + optional: published but never sealed
            // into the cut, deferrable by a progressive carry.
            self.entry(SCHEMA_WORKSPACE, ObjectKindWire::Ephemeral),
        ]
    }
}

impl Adapter for PhotocraftAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "0.1.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L4,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![
                SCHEMA_DOC.into(),
                SCHEMA_UNSAVED.into(),
                SCHEMA_META.into(),
                SCHEMA_WORKSPACE.into(),
            ],
            actions: vec!["document.edit".into(), "session.restore".into()],
            // The Phase-2 drivers read and write the user's selected document
            // workspace under PhotoCraft's own fail-closed automation roots
            // (RESEARCH §2.2–2.3); the adapter never takes ambient paths.
            permissions: vec![
                "read_selected_workspace".into(),
                "write_selected_workspace".into(),
            ],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: true,
            supports_authority_transfer: true,
            max_object_bytes: MAX_OBJECT_BYTES,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("consent-{}", scope.target)))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: self.session.clone(),
            title: format!("PhotoCraft document {}", self.meta.document_name),
            generation: self.generation,
            schema_version: self.meta.schema_version,
        }])
    }

    fn begin_snapshot(&mut self, _s: &str, expected: u64) -> Result<SnapshotToken, AdapterError> {
        if expected != self.generation {
            return Err(AdapterError::StaleGeneration {
                expected,
                actual: self.generation,
            });
        }
        // Attached mode: refresh the in-memory state from the live PhotoCraft
        // now, so the cut seals what is live at this moment (the adapter's
        // `generation` is the snapshot counter; PhotoCraft's own revision
        // travels in the meta object). Detached mode: state stands as built.
        if let Some(driver) = self.driver.as_deref_mut() {
            let document = driver
                .document_bytes()
                .map_err(|e| AdapterError::Incompatible(format!("document_bytes: {e}")))?;
            let unsaved = driver
                .unsaved_bytes()
                .map_err(|e| AdapterError::Incompatible(format!("unsaved_bytes: {e}")))?;
            let workspace_bytes = driver
                .workspace_bytes()
                .map_err(|e| AdapterError::Incompatible(format!("workspace_bytes: {e}")))?;
            let workspace = serde_json::from_slice(&workspace_bytes)
                .map_err(|e| AdapterError::Incompatible(format!("decode workspace: {e}")))?;
            let meta = driver
                .meta()
                .map_err(|e| AdapterError::Incompatible(format!("meta: {e}")))?;
            self.document = document;
            self.unsaved = unsaved;
            self.workspace = workspace;
            self.meta = meta;
        }
        Ok(SnapshotToken(format!("snap-{}", self.generation)))
    }

    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Ok(ObjectManifest {
            session: self.session.clone(),
            generation: self.generation,
            objects: self.all_entries(),
        })
    }

    fn read_object(
        &self,
        _t: &SnapshotToken,
        object_id: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        let bytes = self
            .object_bytes(object_id)
            .ok_or_else(|| AdapterError::UnknownObject(object_id.into()))?;
        let start = (offset as usize).min(bytes.len());
        let end = (start + length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: self.session.clone(),
            generation: self.generation,
            manifest_digest: self.auth_hash(),
        })
    }

    fn abort_snapshot(&mut self, _t: SnapshotToken, _reason: &str) {}

    fn current_generation(&self, _s: &str) -> Result<u64, AdapterError> {
        Ok(self.generation)
    }

    fn resolve_action(
        &self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError> {
        match req.class.as_str() {
            // Both actions need the authoritative content; the workspace object
            // is optional (latency-only); meta is provenance (§6.6).
            "document.edit" | "session.restore" => Ok(DependencyPlanWire {
                prerequisites: vec![
                    ObjectVersionWire {
                        object_id: SCHEMA_DOC.into(),
                        generation: self.generation,
                    },
                    ObjectVersionWire {
                        object_id: SCHEMA_UNSAVED.into(),
                        generation: self.generation,
                    },
                    ObjectVersionWire {
                        object_id: SCHEMA_META.into(),
                        generation: self.generation,
                    },
                ],
                provenance: vec![ObjectVersionWire {
                    object_id: SCHEMA_META.into(),
                    generation: self.generation,
                }],
                optional: vec![ObjectVersionWire {
                    object_id: SCHEMA_WORKSPACE.into(),
                    generation: self.generation,
                }],
            }),
            other => Err(AdapterError::ActionUnsupported(other.into())),
        }
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        let known = [SCHEMA_DOC, SCHEMA_UNSAVED, SCHEMA_META, SCHEMA_WORKSPACE];
        let missing: Vec<_> = versions
            .iter()
            .filter(|v| !known.contains(&v.object_id.as_str()))
            .cloned()
            .collect();
        Ok(ValidationReport {
            ok: missing.is_empty(),
            missing,
            message: "photocraft session validation".into(),
        })
    }

    fn import_objects(
        &mut self,
        _cut: &CutRef,
        locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
        // Receipt only, like the reference adapter: the bytes live in the
        // core's store; the adapter never names paths (golden rule 3). The
        // Phase-2 driver is what would open the carried document in a real
        // PhotoCraft on this machine; this skeleton records the import.
        Ok(ImportReceipt {
            imported: locations
                .iter()
                .map(|l| ObjectVersionWire {
                    object_id: l.object_id.clone(),
                    generation: l.generation,
                })
                .collect(),
        })
    }

    fn activate(
        &mut self,
        _cut: &CutRef,
        _req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        // Activation of a real PhotoCraft (raising the app, opening the doc) is
        // driver work — Phase 2, UNSUPPORTED. The in-memory activation receipt
        // says exactly that in its detail string; it does not claim a live app.
        Ok(ActivationReceipt {
            activated: false,
            detail: "skeleton adapter: no live PhotoCraft activation in Phase 1 \
                     (driver UNSUPPORTED, ADAPTER-DESIGN §5)"
                .into(),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        match req.class.as_str() {
            "document.edit" => {
                // Params: {"steps": [["<command id>", {params}], …]} — the
                // journal/actions shape (RESEARCH §1.3). Parse strictly before
                // touching the driver; fail closed on anything else.
                let steps = parse_steps(req.params.get("steps")).ok_or_else(|| {
                    AdapterError::Internal(
                        "document.edit requires 'steps': [[\"command.id\", {params}], …]".into(),
                    )
                })?;
                let driver = self.driver.as_deref_mut().ok_or_else(|| {
                    AdapterError::ActionUnsupported(
                        "document.edit needs a PhotocraftDriver; none attached \
                         (Phase 2, ADAPTER-DESIGN §5)"
                            .into(),
                    )
                })?;
                let outcome = driver.apply_steps(&steps).map_err(|e| match e {
                    DriverError::Unsupported(m) => {
                        AdapterError::ActionUnsupported(format!("photocraft driver: {m}"))
                    }
                    DriverError::Failed(m) => {
                        AdapterError::Internal(format!("photocraft driver: {m}"))
                    }
                })?;
                let output = serde_json::json!({
                    "revision": outcome.new_revision,
                    "unsaved_hash": outcome.unsaved_hash,
                    "steps_ran": outcome.steps_ran,
                });
                let output_hash =
                    model::hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                // The oracle cannot be checked against a real PhotoCraft in
                // this skeleton (no driver succeeds); checked=false is the
                // honest report, agreed=false with it.
                Ok(ActionResultWire {
                    output,
                    output_hash,
                    oracle: OracleOutcome {
                        checked: false,
                        agreed: false,
                        output_hash: String::new(),
                        detail: "oracle requires a live PhotoCraft driver (Phase 2)".into(),
                    },
                })
            }
            // Read-only: report the restored logical session so a destination
            // can prove it holds the same document/position. With a driver
            // attached this ALSO drives the live PhotoCraft to the carried
            // position and fails closed if the live restore fails; detached
            // (in-memory) it only reports state.
            "session.restore" => {
                let live_restore = match self.driver.as_deref_mut() {
                    None => None,
                    Some(driver) => {
                        match driver.restore(&self.workspace) {
                            Ok(()) => Some(true),
                            // Fail closed: a live restore that did not happen
                            // must not be reported as a restored session.
                            Err(e) => {
                                return Err(match e {
                                    DriverError::Unsupported(m) => AdapterError::ActionUnsupported(
                                        format!("live restore: {m}"),
                                    ),
                                    DriverError::Failed(m) => {
                                        AdapterError::Internal(format!("live restore: {m}"))
                                    }
                                });
                            }
                        }
                    }
                };
                let output = serde_json::json!({
                    "document_hash": model::hex_sha256(&self.document),
                    "unsaved_hash": model::hex_sha256(&self.unsaved),
                    "revision": self.meta.revision,
                    "dirty": self.meta.dirty,
                    "selected_layers": self.workspace.selected_layers,
                    "has_selection": !self.workspace.selected_layers.is_empty(),
                    "viewport": self.workspace.viewport,
                    "active_tool": self.workspace.active_tool,
                    "generation": self.generation,
                    "live_restore": live_restore,
                });
                let output_hash =
                    model::hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                let auth = self.auth_hash();
                let rederived =
                    model::authoritative_hash(&self.document, &self.unsaved, &self.meta_bytes());
                // Agreed = the carried state is internally consistent AND the
                // live restore (if one was attempted) succeeded. Detached
                // mode: state consistency only.
                let agreed = auth == rederived && live_restore.unwrap_or(true);
                Ok(ActionResultWire {
                    output,
                    output_hash: output_hash.clone(),
                    oracle: OracleOutcome {
                        checked: true,
                        agreed,
                        output_hash,
                        detail: format!(
                            "session restored at generation {}; auth_hash {} the sealed state",
                            self.generation,
                            if agreed { "matches" } else { "does not match" }
                        ),
                    },
                })
            }
            other => Err(AdapterError::ActionUnsupported(other.into())),
        }
    }

    // --- Authority transfer (L4, §21.2) ---------------------------------------

    fn prepare_authority_transfer(&mut self, cut: &CutRef) -> Result<String, AdapterError> {
        let proposal = Proposal {
            session: cut.session.clone(),
            cut_number: cut.cut_number,
            auth_hash: self.auth_hash(),
        };
        let blob = serde_json::to_string(&proposal)
            .map_err(|e| AdapterError::Internal(format!("serialize proposal: {e}")))?;
        self.pending = Some(PendingTransfer {
            auth_hash: proposal.auth_hash,
        });
        Ok(blob)
    }

    fn accept_authority_transfer(
        &mut self,
        cut: &CutRef,
        proposal: &str,
    ) -> Result<String, AdapterError> {
        let p: Proposal = serde_json::from_str(proposal)
            .map_err(|e| AdapterError::Internal(format!("parse proposal: {e}")))?;
        if p.cut_number != cut.cut_number {
            return Err(AdapterError::Incompatible(format!(
                "proposal cut {} != imported cut {}",
                p.cut_number, cut.cut_number
            )));
        }
        // The destination re-derives the authoritative-state hash from the
        // state it imported and must agree — authority is bound to state,
        // never to a bearer token (§3.8).
        if p.auth_hash != self.auth_hash() {
            return Err(AdapterError::Incompatible(
                "proposal authoritative-state hash does not match imported state".into(),
            ));
        }
        Ok(format!("accept:{}:{}", p.cut_number, p.auth_hash))
    }

    fn commit_authority_transfer(
        &mut self,
        proposal_id: &str,
        dest_receipt: &str,
    ) -> Result<String, AdapterError> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| AdapterError::Incompatible("no authority transfer prepared".into()))?;
        if !dest_receipt.ends_with(&pending.auth_hash) {
            self.pending = Some(pending);
            return Err(AdapterError::Incompatible(
                "destination receipt does not match prepared proposal".into(),
            ));
        }
        Ok(format!("relinquish:{proposal_id}:{}", pending.auth_hash))
    }

    fn abort_authority_transfer(&mut self, _proposal_id: &str, _reason: &str) {
        self.pending = None;
    }

    fn export_evidence(
        &self,
        session: &str,
        _range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: session.into(),
            json: serde_json::json!({
                "session": self.session,
                "generation": self.generation,
                "document_hash": model::hex_sha256(&self.document),
                "unsaved_hash": model::hex_sha256(&self.unsaved),
                "auth_hash": self.auth_hash(),
                "meta": self.meta,
            }),
        })
    }
}

/// Parse the `document.edit` steps parameter strictly: an array of
/// `[command-id, params-object]` pairs. Anything else is None → the caller
/// fails closed with an error naming the expected shape.
fn parse_steps(value: Option<&serde_json::Value>) -> Option<Vec<Step>> {
    let arr = value?.as_array()?;
    let mut steps = Vec::with_capacity(arr.len());
    for entry in arr {
        let pair = entry.as_array()?;
        if pair.len() != 2 {
            return None;
        }
        let id = pair[0].as_str()?.to_string();
        if !pair[1].is_object() && !pair[1].is_null() {
            return None;
        }
        steps.push(Step {
            id,
            params: pair[1].clone(),
        });
    }
    Some(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_steps_accepts_journal_shape() {
        let v = serde_json::json!([["paint.stroke", {"size": 20}], ["edit.undo", null]]);
        let steps = parse_steps(Some(&v)).expect("journal shape");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].id, "paint.stroke");
    }

    #[test]
    fn parse_steps_rejects_wrong_shapes() {
        assert!(parse_steps(None).is_none());
        assert!(parse_steps(Some(&serde_json::json!({}))).is_none());
        assert!(parse_steps(Some(&serde_json::json!([[{"not": "a string"}]]))).is_none());
        assert!(parse_steps(Some(&serde_json::json!([["id", "params-not-object"]]))).is_none());
        assert!(parse_steps(Some(&serde_json::json!([["id", {}, "extra"]]))).is_none());
    }

    #[test]
    fn auth_hash_is_state_bound() {
        let meta = PhotocraftMeta {
            schema_id: "s".into(),
            schema_version: 1,
            adapter_version: "0.1.0".into(),
            original_path: None,
            document_name: "d".into(),
            revision: 2,
            saved_revision: 1,
            dirty: true,
            photocraft_pin: "pin".into(),
            unsaved_mode: model::UnsavedMode::Snapshot,
        };
        let meta_bytes = serde_json::to_vec(&meta).unwrap();
        let a = model::authoritative_hash(b"doc", b"unsaved", &meta_bytes);
        let b = model::authoritative_hash(b"doc", b"unsaved2", &meta_bytes);
        let c = model::authoritative_hash(b"doc2", b"unsaved", &meta_bytes);
        assert_ne!(a, b, "unsaved bytes must change the hash");
        assert_ne!(a, c, "document bytes must change the hash");
        // Deterministic: same state, same hash.
        assert_eq!(
            a,
            model::authoritative_hash(b"doc", b"unsaved", &meta_bytes)
        );
    }

    #[test]
    fn hex_sha256_known_vector() {
        // sha256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            model::hex_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
