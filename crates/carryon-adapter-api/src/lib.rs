//! Carry-On adapter API (spec §10.5) — the boundary every external-application
//! adapter implements.
//!
//! # Isolation boundary (§7.4, CORE-006)
//!
//! This crate has **no dependency on `carryon-core`**. An adapter therefore
//! cannot name `Store`, `Db`, or `Journal`, and everything crossing the boundary
//! is plain `serde`-serializable data or raw bytes (`Vec<u8>`). The core hashes
//! and stages all bytes itself; an adapter only *serves* bytes. This makes it
//! structurally impossible for an adapter to corrupt core state or publish an
//! object the core did not verify.
//!
//! # Phase 1 scope
//!
//! Adapters are compiled-in Rust, in-process. There is no executable path to
//! resolve — the manifest's `executable` field MUST be the literal
//! `"compiled-in"` (§3.8, no remote code execution; ADP-002 by construction).
//! Network transport, out-of-process IPC, and authority transfer (L4/L5) are
//! Phase 2; the authority-transfer trait methods exist but default to refusal.

use serde::{Deserialize, Serialize};

/// Integration level (§10.2). Phase 1 supports L0, L1, L3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationLevel {
    /// Activation only.
    L0,
    /// File continuation.
    L1,
    /// Navigation state (Phase 2 target, declared but unused in Phase 1 refs).
    L2,
    /// Structured read-only session.
    L3,
    /// Single-writer continuation (Phase 2+).
    L4,
    /// Adapter-defined merge (optional, Phase 2+).
    L5,
}

/// Object kind as declared by an adapter manifest (§6.4). The core re-validates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKindWire {
    Authoritative,
    Derived,
    Cache,
    Preview,
    Ephemeral,
}

/// Sensitivity classification (§6.3/§22.3). `Secret`/`Prohibited` are rejected by
/// the core by default (ADP-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensitivityWire {
    Public,
    Personal,
    Confidential,
    Secret,
    Prohibited,
}

/// Retention policy (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetentionWire {
    Session,
    Bounded,
    Persistent,
    NoCache,
}

/// Static adapter description (spec `GetAdapterInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub adapter_id: String,
    pub adapter_version: String,
    pub publisher_id: String,
    pub integration_level: IntegrationLevel,
    /// MUST be the literal `"compiled-in"` in Phase 1 (no executable path, no RCE).
    pub executable: String,
    pub state_schemas: Vec<String>,
    pub actions: Vec<String>,
    pub permissions: Vec<String>,
    pub network_access: bool,
    pub supports_snapshot: bool,
    pub supports_mutations: bool,
    pub supports_authority_transfer: bool,
    pub max_object_bytes: u64,
}

impl AdapterInfo {
    /// The only legal `executable` value in Phase 1.
    pub const COMPILED_IN: &'static str = "compiled-in";
}

/// Consent scope requested/granted for an adapter session (§10.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentScope {
    pub adapter_id: String,
    /// Logical session or file selection the consent covers.
    pub target: String,
    pub allow_snapshot: bool,
    pub allow_mutations: bool,
}

/// An opaque consent token the core issues after the user grants consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentToken(pub String);

/// Summary of one logical session an adapter exposes (spec `ListSessions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session: String,
    pub title: String,
    pub generation: u64,
    pub schema_version: u32,
}

/// Opaque token identifying an open snapshot transaction (spec `BeginSnapshot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotToken(pub String);

/// One object entry in a snapshot manifest (spec `DescribeSnapshot`). The adapter
/// *declares* these; the core verifies `content_hash`/`logical_size` against the
/// bytes it reads before trusting them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectEntry {
    pub object_id: String,
    pub generation: u64,
    pub kind: ObjectKindWire,
    pub schema_id: String,
    /// Hex sha256 of the exact logical bytes (lowercase, 64 chars).
    pub content_hash: String,
    pub logical_size: u64,
    pub parents: Vec<ObjectVersionWire>,
    pub recipe_id: Option<String>,
    pub portable: bool,
    pub sensitivity: SensitivityWire,
    pub retention: RetentionWire,
}

/// A manifest = ordered object entries for one snapshot/cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectManifest {
    pub session: String,
    pub generation: u64,
    pub objects: Vec<ObjectEntry>,
}

/// Object identity by (id, generation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectVersionWire {
    pub object_id: String,
    pub generation: u64,
}

/// Receipt returned when a snapshot transaction finishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotReceipt {
    pub session: String,
    pub generation: u64,
    pub manifest_digest: String,
}

/// Reference to a sealed cut (session + cut number).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutRef {
    pub session: String,
    pub cut_number: u64,
}

/// An action request: class + JSON parameters (spec `ActionRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequestWire {
    pub class: String,
    pub params: serde_json::Value,
}

/// Dependency plan the adapter returns for an action (§6.6). The core distinguishes
/// execution prerequisites from provenance-only and latency-only objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyPlanWire {
    pub prerequisites: Vec<ObjectVersionWire>,
    pub provenance: Vec<ObjectVersionWire>,
    pub optional: Vec<ObjectVersionWire>,
}

/// Outcome of an adapter's own correctness check for an action (§23.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleOutcome {
    pub checked: bool,
    pub agreed: bool,
    /// Hex sha256 of the canonical output, for evidence.
    pub output_hash: String,
    pub detail: String,
}

/// Result of executing an action (spec `ActionResult`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResultWire {
    pub output: serde_json::Value,
    pub output_hash: String,
    pub oracle: OracleOutcome,
}

/// Report from validating that imported objects satisfy a cut (spec `ValidateObjects`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub missing: Vec<ObjectVersionWire>,
    pub message: String,
}

/// Where an imported object's bytes live locally (content digest + logical size).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectLocation {
    pub object_id: String,
    pub generation: u64,
    pub content_hash: String,
    pub logical_size: u64,
}

/// Receipt confirming the destination imported the objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReceipt {
    pub imported: Vec<ObjectVersionWire>,
}

/// Receipt confirming the destination application was activated for an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationReceipt {
    pub activated: bool,
    pub detail: String,
}

/// A fragment of adapter-sourced evidence (spec `ExportEvidence`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFragment {
    pub session: String,
    pub json: serde_json::Value,
}

/// Range selector for evidence export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceRange {
    All,
}

/// Errors an adapter may return (spec §24 `ADAPTER_*`/`ACTION_*`/`SCHEMA_*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterError {
    /// Adapter cannot satisfy the requested integration level or action.
    Incompatible(String),
    /// Requested action is not supported by this adapter.
    ActionUnsupported(String),
    /// Consent missing or revoked.
    ConsentRequired(String),
    /// Snapshot generation moved; snapshot is stale.
    StaleGeneration { expected: u64, actual: u64 },
    /// Object id unknown or out of range.
    UnknownObject(String),
    /// Caller asked for bytes outside the object.
    OutOfRange(String),
    /// Any other adapter-internal failure.
    Internal(String),
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use AdapterError::*;
        match self {
            Incompatible(m) => write!(f, "ADAPTER_INCOMPATIBLE: {m}"),
            ActionUnsupported(m) => write!(f, "ACTION_UNSUPPORTED: {m}"),
            ConsentRequired(m) => write!(f, "ADAPTER_CONSENT_REQUIRED: {m}"),
            StaleGeneration { expected, actual } => {
                write!(
                    f,
                    "ADAPTER_STALE_GENERATION: expected {expected}, got {actual}"
                )
            }
            UnknownObject(m) => write!(f, "OBJECT_UNKNOWN: {m}"),
            OutOfRange(m) => write!(f, "OBJECT_OUT_OF_RANGE: {m}"),
            Internal(m) => write!(f, "ADAPTER_INTERNAL: {m}"),
        }
    }
}

impl std::error::Error for AdapterError {}

/// The adapter contract (spec §10.5). All methods return **data only**; the core
/// owns persistence, hashing, and verification.
///
/// `Send` is required so the host can hold `Box<dyn Adapter>` and wrap calls in
/// crash containment; adapters are single-threaded in Phase 1.
pub trait Adapter: Send {
    fn get_adapter_info(&self) -> AdapterInfo;

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError>;

    fn list_sessions(&self, consent: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError>;

    fn begin_snapshot(
        &mut self,
        session: &str,
        expected_generation: u64,
    ) -> Result<SnapshotToken, AdapterError>;

    fn describe_snapshot(&self, token: &SnapshotToken) -> Result<ObjectManifest, AdapterError>;

    /// Serve `length` bytes of `object_id` starting at `offset`. The core hashes
    /// and stages these; the adapter never writes to the store.
    fn read_object(
        &self,
        token: &SnapshotToken,
        object_id: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError>;

    fn finish_snapshot(&mut self, token: SnapshotToken) -> Result<SnapshotReceipt, AdapterError>;

    fn abort_snapshot(&mut self, token: SnapshotToken, reason: &str);

    /// Phase 1 polling stand-in for `SubscribeMutations`: report the current
    /// generation so the core can detect staleness.
    fn current_generation(&self, session: &str) -> Result<u64, AdapterError>;

    fn resolve_action(
        &self,
        cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError>;

    fn validate_objects(
        &self,
        cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError>;

    fn import_objects(
        &mut self,
        cut: &CutRef,
        locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError>;

    fn activate(
        &mut self,
        cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError>;

    fn execute_action(
        &mut self,
        cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError>;

    // --- Authority transfer (L4/L5). Phase 1 default: refuse. ---

    fn prepare_authority_transfer(&mut self, _cut: &CutRef) -> Result<String, AdapterError> {
        Err(AdapterError::Incompatible(
            "authority transfer is Phase 2 (L4+)".into(),
        ))
    }

    /// Destination side (§21.2 step 2–3): validate the source's `proposal` against
    /// the imported cut and durably prepare a local commit point, returning the
    /// acceptance receipt. The core persists the receipt before opening the epoch;
    /// the adapter must make its own commit point durable before returning `Ok`.
    fn accept_authority_transfer(
        &mut self,
        _cut: &CutRef,
        _proposal: &str,
    ) -> Result<String, AdapterError> {
        Err(AdapterError::Incompatible(
            "authority transfer is Phase 2 (L4+)".into(),
        ))
    }

    fn commit_authority_transfer(
        &mut self,
        _proposal: &str,
        _dest_receipt: &str,
    ) -> Result<String, AdapterError> {
        Err(AdapterError::Incompatible(
            "authority transfer is Phase 2 (L4+)".into(),
        ))
    }

    fn abort_authority_transfer(&mut self, _proposal: &str, _reason: &str) {}

    fn export_evidence(
        &self,
        session: &str,
        range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError>;
}
