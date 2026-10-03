//! Generic file adapter (L1, spec §26.5.3). Continues a document by content-hash
//! identity: one authoritative object `file.document.v1` = exact file bytes.
//! The action `document.open(uri, line, column)` is activation-only — no
//! in-memory unsaved state is claimed (honest L1, §10.7).

use carryon_adapter_api::*;
use sha2::{Digest as _, Sha256};

/// The file adapter: holds the document bytes and a uri.
pub struct FileAdapter {
    uri: String,
    bytes: Vec<u8>,
    generation: u64,
}

const SCHEMA_DOC: &str = "file.document.v1";
const ADAPTER_ID: &str = "org.carryon.file";

impl FileAdapter {
    pub fn new(uri: impl Into<String>, bytes: Vec<u8>) -> Self {
        FileAdapter {
            uri: uri.into(),
            bytes,
            generation: 1,
        }
    }

    pub fn sample() -> Self {
        FileAdapter::new(
            "file:///tmp/notes.txt",
            b"carry-on sample document".to_vec(),
        )
    }

    /// Open a real file from disk and continue it by content-hash identity (L1).
    /// The uri is the canonical `file://` form of the absolute path; the bytes are
    /// the exact file contents (§10.7 generic file-based applications). No unsaved
    /// in-memory state is claimed — reopening is activation-only.
    pub fn from_path(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let uri = format!("file://{}", abs.to_string_lossy());
        Ok(FileAdapter::new(uri, bytes))
    }

    /// The uri this adapter continues.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    fn entry(&self) -> ObjectEntry {
        ObjectEntry {
            object_id: SCHEMA_DOC.into(),
            generation: self.generation,
            kind: ObjectKindWire::Authoritative,
            schema_id: SCHEMA_DOC.into(),
            content_hash: hex_sha256(&self.bytes),
            logical_size: self.bytes.len() as u64,
            parents: vec![],
            recipe_id: None,
            portable: true,
            sensitivity: SensitivityWire::Public,
            retention: RetentionWire::Persistent,
        }
    }
}

impl Adapter for FileAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L1,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![SCHEMA_DOC.into()],
            actions: vec!["document.open".into()],
            permissions: vec!["user_selected_file".into()],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: false,
            supports_authority_transfer: false,
            max_object_bytes: 16 * 1024 * 1024,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("consent-{}", scope.target)))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: "file-session".into(),
            title: self.uri.clone(),
            generation: self.generation,
            schema_version: 1,
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
            session: "file-session".into(),
            generation: self.generation,
            objects: vec![self.entry()],
        })
    }

    fn read_object(
        &self,
        _t: &SnapshotToken,
        object_id: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        if object_id != SCHEMA_DOC {
            return Err(AdapterError::UnknownObject(object_id.into()));
        }
        let start = (offset as usize).min(self.bytes.len());
        let end = (start + length as usize).min(self.bytes.len());
        Ok(self.bytes[start..end].to_vec())
    }

    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: "file-session".into(),
            generation: self.generation,
            manifest_digest: hex_sha256(&self.bytes),
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
        if req.class != "document.open" {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        Ok(DependencyPlanWire {
            prerequisites: vec![ObjectVersionWire {
                object_id: SCHEMA_DOC.into(),
                generation: self.generation,
            }],
            provenance: vec![],
            optional: vec![],
        })
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        let missing: Vec<_> = versions
            .iter()
            .filter(|v| v.object_id != SCHEMA_DOC)
            .cloned()
            .collect();
        Ok(ValidationReport {
            ok: missing.is_empty(),
            missing,
            message: "file validation".into(),
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
        req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        let uri = req
            .params
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.uri);
        Ok(ActivationReceipt {
            activated: true,
            detail: format!("open-descriptor for {uri}"),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        if req.class != "document.open" {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        let output = serde_json::json!({ "opened": self.uri });
        let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
        Ok(ActionResultWire {
            output,
            output_hash: output_hash.clone(),
            // L1 open is activation-only: no oracle (§6.5 OracleKind::None).
            oracle: OracleOutcome {
                checked: false,
                agreed: true,
                output_hash,
                detail: "activation-only".into(),
            },
        })
    }

    fn export_evidence(
        &self,
        session: &str,
        _range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: session.into(),
            json: serde_json::json!({ "uri": self.uri, "size": self.bytes.len() }),
        })
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
