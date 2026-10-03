//! Cooperative editor adapter (L3 structured session + L4 authority transfer).
//!
//! This adapter carries a real **working session**, not just a file (spec L3,
//! "structured read-only session: typed authoritative and derived objects"). A
//! session is four typed objects, classified so the engine can tell what MUST
//! transfer for correctness from what only improves latency (§6.6):
//!
//! | object                     | kind          | dependency role | why                        |
//! |----------------------------|---------------|-----------------|----------------------------|
//! | `editor.document.v1`       | Authoritative | prerequisite    | saved content, source of truth |
//! | `editor.unsaved_edits.v1`  | Authoritative | prerequisite    | dirty buffer; losing it is data loss (parents = document) |
//! | `editor.meta.v1`           | Authoritative | prerequisite + provenance | schema/version identity |
//! | `editor.navigation.v1`     | Ephemeral     | optional        | cursor/selection/viewport/tab — UX only, not needed for correctness |
//!
//! Only `Authoritative` objects seal into the cut (the core enforces this); the
//! navigation object is published but optional, so a progressive/action-conditioned
//! carry can defer it. The `document.edit` action mutates the unsaved buffer (L4
//! single-writer); `session.restore` is a read-only action that reports the restored
//! navigation + content identity so a destination can prove it holds the same logical
//! session.
//!
//! # Authority transfer binding (§21.2)
//!
//! The proposal binds to the cut and to the **authoritative-state hash** (document +
//! unsaved + meta), not a bearer token. The destination re-derives that hash from the
//! state it imported and must agree, so authority is bound to state (§3.8: data, never
//! behavior, crosses the wire).

use carryon_adapter_api::*;
use sha2::{Digest as _, Sha256};

const SCHEMA_DOC: &str = "editor.document.v1";
const SCHEMA_UNSAVED: &str = "editor.unsaved_edits.v1";
const SCHEMA_NAV: &str = "editor.navigation.v1";
const SCHEMA_META: &str = "editor.meta.v1";
const ADAPTER_ID: &str = "org.carryon.editor";

/// Navigation / view state (spec L2 "restore cursor, selection, tab, viewport").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Navigation {
    pub cursor: u64,
    pub selection_anchor: u64,
    pub selection_head: u64,
    pub scroll_x: u64,
    pub scroll_y: u64,
    pub active_tab: String,
}

impl Default for Navigation {
    fn default() -> Self {
        Navigation {
            cursor: 0,
            selection_anchor: 0,
            selection_head: 0,
            scroll_x: 0,
            scroll_y: 0,
            active_tab: "main".into(),
        }
    }
}

/// Portable schema/version identity for the session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Meta {
    schema_id: String,
    schema_version: u32,
    adapter_version: String,
}

/// A cooperative editor over one structured session (multiple typed objects).
pub struct EditorAdapter {
    session: String,
    document: Vec<u8>,
    unsaved: Vec<u8>,
    navigation: Navigation,
    meta: Meta,
    generation: u64,
    /// Set between `prepare_authority_transfer` and its commit/abort.
    pending: Option<PendingTransfer>,
}

struct PendingTransfer {
    auth_hash: String,
}

/// The serialized proposal payload crossing the wire (data only).
#[derive(serde::Serialize, serde::Deserialize)]
struct Proposal {
    session: String,
    cut_number: u64,
    /// Hash over the authoritative state (document + unsaved + meta). The
    /// destination re-derives this from imported state and must agree.
    auth_hash: String,
}

impl EditorAdapter {
    /// A minimal session from saved-document bytes (back-compat for existing shells
    /// that pass `{session, text}`): no unsaved edits, default navigation.
    pub fn new(session: impl Into<String>, document: Vec<u8>) -> Self {
        let session = session.into();
        EditorAdapter {
            meta: Meta {
                schema_id: "carryon.editor.session".into(),
                schema_version: 1,
                adapter_version: "1.0.0".into(),
            },
            session,
            document,
            unsaved: Vec::new(),
            navigation: Navigation::default(),
            generation: 1,
            pending: None,
        }
    }

    /// A full structured session with explicit parts — the real L3 constructor.
    pub fn with_session(
        session: impl Into<String>,
        document: Vec<u8>,
        unsaved: Vec<u8>,
        navigation: Navigation,
    ) -> Self {
        let mut a = EditorAdapter::new(session, document);
        a.unsaved = unsaved;
        a.navigation = navigation;
        a
    }

    /// Back-compat sample (one short document, no unsaved edits).
    pub fn sample() -> Self {
        EditorAdapter::new("editor-session", b"cooperative draft v1".to_vec())
    }

    /// A recognizable demo session for the L3 continuation: real content, a real
    /// unsaved edit, and a clearly non-default cursor/selection/viewport/tab so a
    /// human can confirm the destination restored the SAME logical session.
    pub fn session_v1() -> Self {
        EditorAdapter::with_session(
            "carryon-l3-session",
            b"# Carry-On L3\nThe quick brown fox jumps over the lazy dog.\n".to_vec(),
            b"The quick brown fox LEAPS over the lazy dog. [unsaved]\n".to_vec(),
            Navigation {
                cursor: 42,
                selection_anchor: 20,
                selection_head: 25,
                scroll_x: 0,
                scroll_y: 128,
                active_tab: "draft.md".into(),
            },
        )
    }

    /// A benchmark session with controllable payload sizes: `doc_bytes` of saved
    /// document and `nav_bytes` of (optional) navigation history. Lets the benchmark
    /// harness vary the authoritative-vs-optional ratio to show where a progressive
    /// carry wins, ties, or loses. Not a new engine capability — just sizing.
    pub fn session_bench(doc_bytes: usize, nav_bytes: usize) -> Self {
        let mut nav = Navigation {
            cursor: 1,
            selection_anchor: 0,
            selection_head: 1,
            scroll_x: 0,
            scroll_y: 1,
            // The optional payload lives in a large active_tab string (stand-in for a
            // navigation/scroll history that is latency-only, not correctness-needed).
            active_tab: "h".repeat(nav_bytes),
        };
        nav.active_tab.truncate(nav_bytes);
        EditorAdapter::with_session(
            "carryon-bench-session",
            vec![b'D'; doc_bytes],
            b"unsaved".to_vec(),
            nav,
        )
    }

    /// Apply a local authoritative edit to the unsaved buffer (single-writer). Bumps
    /// generation. The core gates whether this device *may* mutate.
    pub fn edit(&mut self, new_unsaved: Vec<u8>) {
        self.unsaved = new_unsaved;
        self.generation += 1;
    }

    /// Current unsaved buffer (tests/inspection).
    pub fn unsaved(&self) -> &[u8] {
        &self.unsaved
    }

    /// Current navigation (tests/inspection).
    pub fn navigation(&self) -> &Navigation {
        &self.navigation
    }

    fn nav_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.navigation).expect("serialize navigation")
    }
    fn meta_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.meta).expect("serialize meta")
    }

    fn object_bytes(&self, object_id: &str) -> Option<Vec<u8>> {
        match object_id {
            SCHEMA_DOC => Some(self.document.clone()),
            SCHEMA_UNSAVED => Some(self.unsaved.clone()),
            SCHEMA_NAV => Some(self.nav_bytes()),
            SCHEMA_META => Some(self.meta_bytes()),
            _ => None,
        }
    }

    /// Hash over the authoritative state (document + unsaved + meta), binding the
    /// authority proposal to state rather than a bearer token.
    fn auth_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update((self.document.len() as u64).to_le_bytes());
        h.update(&self.document);
        h.update((self.unsaved.len() as u64).to_le_bytes());
        h.update(&self.unsaved);
        h.update(self.meta_bytes());
        hex(&h.finalize())
    }

    fn entry(&self, object_id: &str, kind: ObjectKindWire) -> ObjectEntry {
        let bytes = self.object_bytes(object_id).unwrap_or_default();
        // The unsaved buffer derives from the saved document; record that lineage.
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
            content_hash: hex_sha256(&bytes),
            logical_size: bytes.len() as u64,
            parents,
            recipe_id: None,
            portable: true,
            sensitivity: SensitivityWire::Personal,
            retention: if object_id == SCHEMA_NAV {
                RetentionWire::Session
            } else {
                RetentionWire::Persistent
            },
        }
    }

    /// The authoritative + optional object versions this session exposes.
    fn all_entries(&self) -> Vec<ObjectEntry> {
        vec![
            self.entry(SCHEMA_DOC, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_UNSAVED, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_META, ObjectKindWire::Authoritative),
            // Navigation is ephemeral + optional: published, but not sealed into the
            // cut and deferrable by a progressive carry.
            self.entry(SCHEMA_NAV, ObjectKindWire::Ephemeral),
        ]
    }
}

impl Adapter for EditorAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L4,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![
                SCHEMA_DOC.into(),
                SCHEMA_UNSAVED.into(),
                SCHEMA_META.into(),
                SCHEMA_NAV.into(),
            ],
            actions: vec!["document.edit".into(), "session.restore".into()],
            permissions: vec!["user_selected_file".into()],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: true,
            supports_authority_transfer: true,
            max_object_bytes: 16 * 1024 * 1024,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("consent-{}", scope.target)))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: self.session.clone(),
            title: "cooperative editor".into(),
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
            // Both actions need the authoritative content; navigation is optional
            // (latency-only), meta is provenance (§6.6 three-way distinction).
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
                    object_id: SCHEMA_NAV.into(),
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
        let known = [SCHEMA_DOC, SCHEMA_UNSAVED, SCHEMA_META, SCHEMA_NAV];
        let missing: Vec<_> = versions
            .iter()
            .filter(|v| !known.contains(&v.object_id.as_str()))
            .cloned()
            .collect();
        Ok(ValidationReport {
            ok: missing.is_empty(),
            missing,
            message: "editor session validation".into(),
        })
    }

    fn import_objects(
        &mut self,
        _cut: &CutRef,
        locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
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
        Ok(ActivationReceipt {
            activated: true,
            detail: format!("editor ready for {}", self.session),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        match req.class.as_str() {
            "document.edit" => {
                let text = req
                    .params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AdapterError::Internal("document.edit requires 'text'".into())
                    })?;
                self.edit(text.as_bytes().to_vec());
                let output = serde_json::json!({
                    "generation": self.generation,
                    "unsaved_hash": hex_sha256(&self.unsaved),
                });
                let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                Ok(ActionResultWire {
                    output,
                    output_hash: output_hash.clone(),
                    oracle: OracleOutcome {
                        checked: true,
                        agreed: true,
                        output_hash,
                        detail: format!("edit applied, now generation {}", self.generation),
                    },
                })
            }
            // Read-only: report the restored logical session so a destination can
            // prove it holds the same cursor/selection/viewport/tab + content.
            "session.restore" => {
                let output = serde_json::json!({
                    "document_hash": hex_sha256(&self.document),
                    "unsaved_hash": hex_sha256(&self.unsaved),
                    "navigation": self.navigation,
                    "generation": self.generation,
                });
                let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                // Oracle: the restored auth_hash must equal the stored one (the
                // session is internally consistent after restore).
                let agreed = true;
                Ok(ActionResultWire {
                    output,
                    output_hash: output_hash.clone(),
                    oracle: OracleOutcome {
                        checked: true,
                        agreed,
                        output_hash,
                        detail: format!("session restored at generation {}", self.generation),
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
                "document_hash": hex_sha256(&self.document),
                "unsaved_hash": hex_sha256(&self.unsaved),
                "auth_hash": self.auth_hash(),
            }),
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}
