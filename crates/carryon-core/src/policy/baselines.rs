//! Baseline preparation policies (spec §20.1) plus the default product policy
//! (§20.2). Phase 1 ships full-transfer and demand baselines and the default.

use super::{JobKind, Observation, PrepMode, PreparationPolicy, Proposal, RationaleCode};
use uuid::Uuid;

/// Deterministic proposal id derived from the object + generation so tests are
/// reproducible (no randomness in a policy).
fn proposal_id(object_id: &str, generation: u64) -> Uuid {
    let digest = crate::ids::Digest::of(format!("{object_id}:{generation}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.0[..16]);
    Uuid::from_bytes(bytes)
}

/// §20.1.1 — propose transferring every selected object not already present.
pub struct FullTransferPolicy;

impl PreparationPolicy for FullTransferPolicy {
    fn name(&self) -> &str {
        "full_transfer"
    }

    fn propose(&self, obs: &Observation) -> Vec<Proposal> {
        obs.objects
            .iter()
            .filter(|o| !o.locally_present)
            .map(|o| Proposal {
                proposal_id: proposal_id(&o.id.0, o.generation),
                observation_generation: obs.observation_generation,
                job_kind: JobKind::Transfer,
                object_id: o.id.clone(),
                object_version: o.generation,
                mode: PrepMode::DirectBytes,
                estimated_network_bytes: o.logical_size,
                estimated_cpu_millis: 1,
                estimated_peak_memory: o.logical_size,
                estimated_nonpreemptible_millis: 1,
                expected_action_benefit: if o.required_authoritative { 1.0 } else { 0.5 },
                rationale_code: if o.required_authoritative {
                    RationaleCode::AuthoritativeRequired
                } else {
                    RationaleCode::SharedReuse
                },
            })
            .collect()
    }
}

/// §20.1.3 — pure demand loading: propose nothing up front; everything deferred.
pub struct DemandPolicy;

impl PreparationPolicy for DemandPolicy {
    fn name(&self) -> &str {
        "demand"
    }

    fn propose(&self, obs: &Observation) -> Vec<Proposal> {
        obs.objects
            .iter()
            .filter(|o| !o.locally_present)
            .map(|o| Proposal {
                proposal_id: proposal_id(&o.id.0, o.generation),
                observation_generation: obs.observation_generation,
                job_kind: JobKind::Wait,
                object_id: o.id.clone(),
                object_version: o.generation,
                mode: PrepMode::Defer,
                estimated_network_bytes: 0,
                estimated_cpu_millis: 0,
                estimated_peak_memory: 0,
                estimated_nonpreemptible_millis: 0,
                expected_action_benefit: 0.0,
                rationale_code: RationaleCode::DemandDeferred,
            })
            .collect()
    }
}

/// §20.2 — default product policy: authoritative-required first (transfer), the
/// rest deferred to demand.
pub struct DefaultPolicy;

impl PreparationPolicy for DefaultPolicy {
    fn name(&self) -> &str {
        "default"
    }

    fn propose(&self, obs: &Observation) -> Vec<Proposal> {
        obs.objects
            .iter()
            .filter(|o| !o.locally_present)
            .map(|o| {
                if o.required_authoritative {
                    Proposal {
                        proposal_id: proposal_id(&o.id.0, o.generation),
                        observation_generation: obs.observation_generation,
                        job_kind: JobKind::Transfer,
                        object_id: o.id.clone(),
                        object_version: o.generation,
                        mode: PrepMode::DirectBytes,
                        estimated_network_bytes: o.logical_size,
                        estimated_cpu_millis: 1,
                        estimated_peak_memory: o.logical_size,
                        estimated_nonpreemptible_millis: 1,
                        expected_action_benefit: 1.0,
                        rationale_code: RationaleCode::AuthoritativeRequired,
                    }
                } else {
                    Proposal {
                        proposal_id: proposal_id(&o.id.0, o.generation),
                        observation_generation: obs.observation_generation,
                        job_kind: JobKind::Wait,
                        object_id: o.id.clone(),
                        object_version: o.generation,
                        mode: PrepMode::Defer,
                        estimated_network_bytes: 0,
                        estimated_cpu_millis: 0,
                        estimated_peak_memory: 0,
                        estimated_nonpreemptible_millis: 0,
                        expected_action_benefit: 0.3,
                        rationale_code: RationaleCode::DemandDeferred,
                    }
                }
            })
            .collect()
    }
}
