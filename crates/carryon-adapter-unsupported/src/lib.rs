//! Honest-refusal demo adapter (spec §26.5.6, §30). It advertises an integration
//! level that cannot satisfy a structured snapshot/action, and refuses cleanly
//! with `ADAPTER_INCOMPATIBLE`/`ACTION_UNSUPPORTED` rather than faking readiness.

use carryon_adapter_api::*;

/// An adapter that supports activation only (L0) and cannot export typed state.
pub struct UnsupportedAdapter;

const ADAPTER_ID: &str = "org.carryon.unsupported";

impl Default for UnsupportedAdapter {
    fn default() -> Self {
        UnsupportedAdapter
    }
}

impl Adapter for UnsupportedAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L0, // activation only
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![],
            actions: vec![],
            permissions: vec![],
            network_access: false,
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
        Ok(vec![])
    }

    fn begin_snapshot(&mut self, _s: &str, _g: u64) -> Result<SnapshotToken, AdapterError> {
        Err(AdapterError::Incompatible(
            "this application exposes no typed state; structured continuation is not supported"
                .into(),
        ))
    }

    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Err(AdapterError::Incompatible("no snapshot available".into()))
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
        Err(AdapterError::Incompatible("no snapshot available".into()))
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
        Err(AdapterError::ActionUnsupported(req.class.clone()))
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        _versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        Err(AdapterError::Incompatible("no typed objects".into()))
    }

    fn import_objects(
        &mut self,
        _cut: &CutRef,
        _locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
        Err(AdapterError::Incompatible("nothing to import".into()))
    }

    fn activate(
        &mut self,
        _cut: &CutRef,
        _req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        // L0 can at most open the app; it cannot continue a structured action.
        Ok(ActivationReceipt {
            activated: true,
            detail: "opened application (no state)".into(),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        Err(AdapterError::ActionUnsupported(req.class.clone()))
    }

    fn export_evidence(
        &self,
        session: &str,
        _range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: session.into(),
            json: serde_json::json!({ "supported": false }),
        })
    }
}
