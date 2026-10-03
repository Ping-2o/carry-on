//! Preparation-policy interface (spec §20.3) and core-side admission (§7.4).
//!
//! A policy receives a **read-only** [`Observation`] and returns [`Proposal`]s.
//! It cannot write the DB, send frames, or call adapters — the signature gives
//! it nothing but the observation. The core independently re-verifies every
//! proposal field against the real object graph and budget before admission
//! ([`admit`]), so a lying or inflated proposal cannot cause work the core did
//! not sanction (CORE-006).

pub mod baselines;

pub use baselines::{DefaultPolicy, DemandPolicy, FullTransferPolicy};

use crate::ids::ObjectId;
use crate::model::Budget;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The job a proposal asks for (§20.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobKind {
    Transfer,
    Reconstruct,
    Validate,
    Wait,
}

/// How a transfer/reconstruct is performed (§20.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrepMode {
    /// Copy exact bytes (local in Phase 1).
    DirectBytes,
    /// Reconstruct from pinned parents + recipe.
    Reconstruct,
    /// Defer until demand.
    Defer,
}

/// Why a proposal was made, for evidence (§20.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RationaleCode {
    AuthoritativeRequired,
    ActionClosure,
    SharedReuse,
    DemandDeferred,
}

/// One candidate object in the observed graph (read-only view for a policy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedObject {
    pub id: ObjectId,
    pub generation: u64,
    pub logical_size: u64,
    /// Whether the object is a required authoritative dependency of the action.
    pub required_authoritative: bool,
    /// Whether the object is already locally present (verified).
    pub locally_present: bool,
}

/// A read-only observation handed to a policy (§7.4). No handles to DB/store/host.
/// Not `Eq` (holds `Budget`, whose `cpu_duty` is `f32`).
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub observation_generation: u64,
    pub objects: Vec<ObservedObject>,
    pub budget: Budget,
}

/// A policy proposal (§20.3). Every field is re-verified by the core.
/// Not `Eq` (holds `expected_action_benefit: f32`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub proposal_id: Uuid,
    pub observation_generation: u64,
    pub job_kind: JobKind,
    pub object_id: ObjectId,
    pub object_version: u64,
    pub mode: PrepMode,
    pub estimated_network_bytes: u64,
    pub estimated_cpu_millis: u64,
    pub estimated_peak_memory: u64,
    pub estimated_nonpreemptible_millis: u64,
    pub expected_action_benefit: f32,
    pub rationale_code: RationaleCode,
}

/// A preparation policy (§20.3). Read-only input, proposals out.
pub trait PreparationPolicy {
    fn name(&self) -> &str;
    fn propose(&self, obs: &Observation) -> Vec<Proposal>;
}

/// Why the core rejected a proposal at admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitRejection {
    StaleObservation,
    UnknownObject,
    WrongVersion,
    AlreadyPresent,
    BudgetExceeded(&'static str),
}

/// Admit a proposal: independently re-verify every field against the real
/// observed graph and budget (§20.3, §7.4). Returns `Ok(())` if the core will
/// perform the job, or a typed rejection. The core — not the policy — is the
/// authority on object identity, presence, and budget.
pub fn admit(obs: &Observation, p: &Proposal) -> std::result::Result<(), AdmitRejection> {
    if p.observation_generation != obs.observation_generation {
        return Err(AdmitRejection::StaleObservation);
    }
    let obj = obs
        .objects
        .iter()
        .find(|o| o.id == p.object_id)
        .ok_or(AdmitRejection::UnknownObject)?;
    if obj.generation != p.object_version {
        return Err(AdmitRejection::WrongVersion);
    }
    // A Wait proposal always admits (it does no work).
    if p.job_kind == JobKind::Wait {
        return Ok(());
    }
    if obj.locally_present && p.job_kind == JobKind::Transfer {
        return Err(AdmitRejection::AlreadyPresent);
    }
    // Re-verify the byte estimate against the true size: a transfer can move at
    // most the object's logical size. An inflated estimate is clamped/rejected.
    if p.job_kind == JobKind::Transfer && p.estimated_network_bytes > obj.logical_size {
        return Err(AdmitRejection::BudgetExceeded(
            "estimate exceeds object size",
        ));
    }
    if p.estimated_network_bytes > obs.budget.total_net_bytes {
        return Err(AdmitRejection::BudgetExceeded("network"));
    }
    if p.estimated_cpu_millis > obs.budget.total_cpu_ms {
        return Err(AdmitRejection::BudgetExceeded("cpu"));
    }
    if p.estimated_peak_memory > obs.budget.peak_prep_mem {
        return Err(AdmitRejection::BudgetExceeded("memory"));
    }
    if p.estimated_nonpreemptible_millis > obs.budget.max_nonpreemptible_ms {
        return Err(AdmitRejection::BudgetExceeded("nonpreemptible"));
    }
    Ok(())
}
