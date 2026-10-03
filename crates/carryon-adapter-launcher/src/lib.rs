//! Universal launcher adapter (L0, spec §10.2). This is "wide support" done
//! honestly: it can produce an **activation descriptor** to open *any* external
//! application, URI, or file — a browser, a PDF viewer, a game, anything the OS can
//! launch — without claiming to capture or transfer that app's state.
//!
//! # What L0 is and is not (spec §4.4, §10.1-10.2)
//!
//! - It **is** a universal "open the destination app/resource" capability. The
//!   allowed handoff claim is exactly "Opened destination app/resource".
//! - It is **not** process/memory migration, save-file scraping, screenshot/OCR,
//!   or reading another app's private container. Carry-On does not promise
//!   arbitrary application-state capture (§10.1), and this adapter embodies that
//!   boundary: it exports **no** authoritative objects, so there is nothing to
//!   misrepresent as "continued state."
//!
//! The activation descriptor names a target by **URI or file path only** — never a
//! shell command or executable path chosen by a remote peer (§3.8, no RCE). The
//! actual OS launch is the platform shell's job (`NSWorkspace`/`xdg-open`/
//! `ShellExecute`/`Intent`); this adapter emits the validated descriptor.

use carryon_adapter_api::*;
use sha2::{Digest as _, Sha256};

const ADAPTER_ID: &str = "org.carryon.launcher";
/// The activation descriptor "schema" — note it is metadata, not transferred bytes.
const ACTION_OPEN: &str = "app.open";

/// A universal activation target. Exactly one of `uri` / `file` is set.
#[derive(Debug, Clone)]
pub struct LaunchTarget {
    /// A URI to open (http(s), custom scheme, mailto, deep link, …).
    pub uri: Option<String>,
    /// An absolute file path to open with its default handler.
    pub file: Option<String>,
    /// Optional human label for the destination (shown in UI only).
    pub label: String,
}

impl LaunchTarget {
    pub fn uri(uri: impl Into<String>) -> Self {
        LaunchTarget {
            uri: Some(uri.into()),
            file: None,
            label: "open uri".into(),
        }
    }

    pub fn file(path: impl Into<String>) -> Self {
        LaunchTarget {
            uri: None,
            file: Some(path.into()),
            label: "open file".into(),
        }
    }
}

/// The L0 launcher adapter. Holds nothing but a default target; every action
/// request may override the target by its JSON params.
pub struct LauncherAdapter {
    default: LaunchTarget,
}

impl LauncherAdapter {
    pub fn new(default: LaunchTarget) -> Self {
        LauncherAdapter { default }
    }

    /// Resolve the effective target from an action request's params, falling back
    /// to the default. Validates the target shape (exactly one of uri/file) and
    /// refuses anything that looks like a shell command or executable selection.
    fn resolve_target(&self, req: &ActionRequestWire) -> Result<LaunchTarget, AdapterError> {
        if req.class != ACTION_OPEN {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        let uri = req.params.get("uri").and_then(|v| v.as_str());
        let file = req.params.get("file").and_then(|v| v.as_str());
        // A remote peer MUST NOT smuggle an executable/command (§3.8). Reject any
        // `command`/`exec`/`argv` field outright.
        for banned in ["command", "exec", "argv", "shell"] {
            if req.params.get(banned).is_some() {
                return Err(AdapterError::Incompatible(format!(
                    "launcher refuses '{banned}': no remote code execution"
                )));
            }
        }
        match (uri, file) {
            (Some(u), None) => Ok(LaunchTarget::uri(u)),
            (None, Some(f)) => Ok(LaunchTarget::file(f)),
            (None, None) => Ok(self.default.clone()),
            (Some(_), Some(_)) => Err(AdapterError::Incompatible(
                "specify exactly one of uri or file".into(),
            )),
        }
    }

    /// The validated, platform-agnostic descriptor the shell will execute. It names
    /// a resource, never a program to run.
    fn descriptor(target: &LaunchTarget) -> serde_json::Value {
        if let Some(uri) = &target.uri {
            serde_json::json!({ "open": "uri", "target": uri })
        } else if let Some(file) = &target.file {
            serde_json::json!({ "open": "file", "target": file })
        } else {
            serde_json::json!({ "open": "none" })
        }
    }
}

impl Adapter for LauncherAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L0,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![],
            actions: vec![ACTION_OPEN.into()],
            permissions: vec!["launch_application".into()],
            network_access: false,
            // L0 has no state: no snapshot, no mutations, no authority.
            supports_snapshot: false,
            supports_mutations: false,
            supports_authority_transfer: false,
            max_object_bytes: 0,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("consent-{}", scope.target)))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: "launcher".into(),
            title: self.default.label.clone(),
            generation: 0,
            schema_version: 1,
        }])
    }

    fn begin_snapshot(&mut self, _s: &str, _expected: u64) -> Result<SnapshotToken, AdapterError> {
        // L0 exports no objects; a snapshot is an empty manifest.
        Ok(SnapshotToken("launcher-empty".into()))
    }

    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Ok(ObjectManifest {
            session: "launcher".into(),
            generation: 0,
            objects: vec![], // nothing to transfer — honest L0
        })
    }

    fn read_object(
        &self,
        _t: &SnapshotToken,
        object_id: &str,
        _offset: u64,
        _length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        Err(AdapterError::UnknownObject(object_id.into()))
    }

    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: "launcher".into(),
            generation: 0,
            manifest_digest: hex_sha256(b""),
        })
    }

    fn abort_snapshot(&mut self, _t: SnapshotToken, _reason: &str) {}

    fn current_generation(&self, _s: &str) -> Result<u64, AdapterError> {
        Ok(0)
    }

    fn resolve_action(
        &self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError> {
        // Activation has no object dependencies (nothing must arrive first).
        self.resolve_target(req)?;
        Ok(DependencyPlanWire {
            prerequisites: vec![],
            provenance: vec![],
            optional: vec![],
        })
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        // No objects are ever part of an L0 cut; any version is spurious.
        Ok(ValidationReport {
            ok: versions.is_empty(),
            missing: vec![],
            message: "launcher has no objects".into(),
        })
    }

    fn import_objects(
        &mut self,
        _cut: &CutRef,
        _locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
        Ok(ImportReceipt { imported: vec![] })
    }

    fn activate(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        let target = self.resolve_target(req)?;
        let descriptor = LauncherAdapter::descriptor(&target);
        Ok(ActivationReceipt {
            activated: true,
            detail: descriptor.to_string(),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        let target = self.resolve_target(req)?;
        let output = LauncherAdapter::descriptor(&target);
        let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
        Ok(ActionResultWire {
            output,
            output_hash: output_hash.clone(),
            // Activation-only: no correctness oracle (§6.5 OracleKind::None).
            oracle: OracleOutcome {
                checked: false,
                agreed: true,
                output_hash,
                detail: "activation-only (L0)".into(),
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
            json: serde_json::json!({
                "integration_level": "L0",
                "claim": "Opened destination app/resource",
                "captures_state": false
            }),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cut() -> CutRef {
        CutRef {
            session: "launcher".into(),
            cut_number: 0,
        }
    }

    #[test]
    fn opens_any_uri() {
        let mut a = LauncherAdapter::new(LaunchTarget::uri("https://example.com"));
        let req = ActionRequestWire {
            class: ACTION_OPEN.into(),
            params: serde_json::json!({ "uri": "steam://run/440" }),
        };
        let r = a.execute_action(&cut(), &req).unwrap();
        assert!(r.output.to_string().contains("steam://run/440"));
    }

    #[test]
    fn refuses_remote_command() {
        let mut a = LauncherAdapter::new(LaunchTarget::uri("https://example.com"));
        let req = ActionRequestWire {
            class: ACTION_OPEN.into(),
            params: serde_json::json!({ "command": "rm -rf /" }),
        };
        let err = a.execute_action(&cut(), &req).unwrap_err();
        assert!(matches!(err, AdapterError::Incompatible(_)));
    }

    #[test]
    fn exports_no_objects() {
        let a = LauncherAdapter::new(LaunchTarget::file("/tmp/x"));
        let m = a.describe_snapshot(&SnapshotToken("t".into())).unwrap();
        assert!(m.objects.is_empty(), "L0 must not export state objects");
    }

    #[test]
    fn unsupported_action_refused() {
        let mut a = LauncherAdapter::new(LaunchTarget::uri("https://x"));
        let req = ActionRequestWire {
            class: "graph.shortest_path".into(),
            params: serde_json::json!({}),
        };
        assert!(matches!(
            a.execute_action(&cut(), &req),
            Err(AdapterError::ActionUnsupported(_))
        ));
    }
}
