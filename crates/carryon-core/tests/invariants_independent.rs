//! Core invariants hold independently of adapter behavior (§7.4, CORE-006,
//! ADP-007/008): a lying, oversizing, secret-emitting, or panicking adapter
//! cannot corrupt the store or DB.

use carryon_adapter_api::*;
use carryon_core::model::{AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

/// Shared skeleton for a one-object adapter whose single object is configurable.
struct EvilAdapter {
    entry: ObjectEntry,
    bytes: Vec<u8>,
    panic_on_read: bool,
}

impl EvilAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: "org.carryon.evil".into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L3,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec!["evil.v1".into()],
            actions: vec![],
            permissions: vec![],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: false,
            supports_authority_transfer: false,
            max_object_bytes: 16 * 1024 * 1024,
        }
    }
}

impl Adapter for EvilAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        self.info()
    }
    fn request_consent(&mut self, s: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("c-{}", s.target)))
    }
    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: "evil".into(),
            title: "evil".into(),
            generation: 1,
            schema_version: 1,
        }])
    }
    fn begin_snapshot(&mut self, _s: &str, _g: u64) -> Result<SnapshotToken, AdapterError> {
        Ok(SnapshotToken("snap".into()))
    }
    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Ok(ObjectManifest {
            session: "evil".into(),
            generation: 1,
            objects: vec![self.entry.clone()],
        })
    }
    fn read_object(
        &self,
        _t: &SnapshotToken,
        _o: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        if self.panic_on_read {
            panic!("evil adapter panics mid-read");
        }
        let start = (offset as usize).min(self.bytes.len());
        let end = (start + length as usize).min(self.bytes.len());
        Ok(self.bytes[start..end].to_vec())
    }
    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: "evil".into(),
            generation: 1,
            manifest_digest: "00".into(),
        })
    }
    fn abort_snapshot(&mut self, _t: SnapshotToken, _r: &str) {}
    fn current_generation(&self, _s: &str) -> Result<u64, AdapterError> {
        Ok(1)
    }
    fn resolve_action(
        &self,
        _c: &CutRef,
        r: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError> {
        Err(AdapterError::ActionUnsupported(r.class.clone()))
    }
    fn validate_objects(
        &self,
        _c: &CutRef,
        _v: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        Ok(ValidationReport {
            ok: true,
            missing: vec![],
            message: String::new(),
        })
    }
    fn import_objects(
        &mut self,
        _c: &CutRef,
        _l: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
        Ok(ImportReceipt { imported: vec![] })
    }
    fn activate(
        &mut self,
        _c: &CutRef,
        _r: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        Ok(ActivationReceipt {
            activated: true,
            detail: String::new(),
        })
    }
    fn execute_action(
        &mut self,
        _c: &CutRef,
        r: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        Err(AdapterError::ActionUnsupported(r.class.clone()))
    }
    fn export_evidence(
        &self,
        s: &str,
        _r: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: s.into(),
            json: serde_json::Value::Null,
        })
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn entry(schema: &str, hash: &str, size: u64, sens: SensitivityWire) -> ObjectEntry {
    ObjectEntry {
        object_id: "evil.v1".into(),
        generation: 1,
        kind: ObjectKindWire::Authoritative,
        schema_id: schema.into(),
        content_hash: hash.into(),
        logical_size: size,
        parents: vec![],
        recipe_id: None,
        portable: true,
        sensitivity: sens,
        retention: RetentionWire::Session,
    }
}

fn run(adapter: EvilAdapter) -> (tempfile::TempDir, carryon_core::CoreError) {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core.register_adapter(Box::new(adapter)).unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "evil".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let err = core.create_cut(session).unwrap_err();
    (dir, err)
}

#[test]
fn lying_hash_fails_closed() {
    // Declares the hash of "truth" but serves "tamper": digest mismatch.
    let truth = b"truth".to_vec();
    let adapter = EvilAdapter {
        entry: entry("evil.v1", &hex_sha256(&truth), 5, SensitivityWire::Public),
        bytes: b"tamper!".to_vec(),
        panic_on_read: false,
    };
    let (_dir, err) = run(adapter);
    assert_eq!(err.family(), "OBJECT");
}

#[test]
fn secret_object_is_excluded() {
    let bytes = b"api-key-123".to_vec();
    let adapter = EvilAdapter {
        entry: entry(
            "evil.v1",
            &hex_sha256(&bytes),
            bytes.len() as u64,
            SensitivityWire::Secret,
        ),
        bytes,
        panic_on_read: false,
    };
    let (_dir, err) = run(adapter);
    assert_eq!(err.family(), "OBJECT");
    assert!(
        err.to_string().contains("excluded"),
        "secret must be excluded by default (ADP-008)"
    );
}

#[test]
fn oversized_object_is_rejected() {
    // Declare a logical size over the core cap (16 MiB).
    let adapter = EvilAdapter {
        entry: entry(
            "evil.v1",
            &hex_sha256(b"x"),
            100 * 1024 * 1024,
            SensitivityWire::Public,
        ),
        bytes: b"x".to_vec(),
        panic_on_read: false,
    };
    let (_dir, err) = run(adapter);
    assert_eq!(err.family(), "OBJECT");
}

#[test]
fn panicking_adapter_is_contained() {
    // A panic mid-read must be contained as ADAPTER_Crashed, not unwind the core.
    let bytes = b"data".to_vec();
    let adapter = EvilAdapter {
        entry: entry(
            "evil.v1",
            &hex_sha256(&bytes),
            bytes.len() as u64,
            SensitivityWire::Public,
        ),
        bytes,
        panic_on_read: true,
    };
    let (_dir, err) = run(adapter);
    assert_eq!(err.family(), "ADAPTER", "panic must be contained (ADP-007)");
}
