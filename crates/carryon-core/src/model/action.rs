//! Action contracts and dependency closures (spec §6.5/§6.6).

use crate::ids::{Digest, ObjectId, ObjectVersion};
use serde::{Deserialize, Serialize};

/// What correctness check an action's result is subject to (§23.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OracleKind {
    /// Two independent computations must agree.
    DualComputation,
    /// Output must match a pinned golden hash.
    Golden,
    /// No oracle (activation-only actions).
    None,
}

/// How the destination application is activated for an action (§6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationSpec {
    /// A declared, documented mechanism string (e.g. `"open-descriptor"`).
    /// MUST NOT be an executable path or shell string (§3.8).
    pub mechanism: String,
}

/// Where latency/resource measurement starts and stops (§6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementSpec {
    pub from_request: bool,
    pub to_action_finish: bool,
}

/// A declared action contract (spec §6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub class: String,
    pub param_schema: serde_json::Value,
    pub resolver_version: String,
    pub required_authoritative: Vec<ObjectId>,
    pub optional_derived: Vec<ObjectId>,
    pub output_schema: serde_json::Value,
    pub oracle: OracleKind,
    pub mutates_authoritative: bool,
    pub activation: ActivationSpec,
    pub measurement_boundaries: MeasurementSpec,
}

/// A concrete request against an action class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub class: String,
    pub params: serde_json::Value,
}

/// Resolved dependency closure for an action at a cut (spec §6.6). The engine
/// distinguishes execution prerequisites, validation provenance, and
/// latency-only optional data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyPlan {
    pub prerequisites: Vec<ObjectVersion>,
    pub provenance: Vec<ObjectVersion>,
    pub optional: Vec<ObjectVersion>,
}

/// Correctness outcome recorded for an executed action (§23.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleOutcome {
    pub checked: bool,
    pub agreed: bool,
    pub output_hash: Digest,
    pub detail: String,
}

/// Result of executing an action (spec `ActionResult`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub output: serde_json::Value,
    pub output_hash: Digest,
    pub oracle: OracleOutcome,
}

/// Whether an action can run now at a cut, and why not if not (§16.1 Continue).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionAvailability {
    pub class: String,
    pub ready: bool,
    pub missing: Vec<ObjectVersion>,
}
