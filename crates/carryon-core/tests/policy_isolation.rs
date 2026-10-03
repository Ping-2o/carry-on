//! Policy isolation + admission (§7.4, §20.3, CORE-006). A policy only sees a
//! read-only Observation; the core re-verifies every proposal and rejects
//! inflated/stale/already-present ones.

use carryon_core::ids::ObjectId;
use carryon_core::model::Budget;
use carryon_core::policy::{
    admit, AdmitRejection, DefaultPolicy, DemandPolicy, FullTransferPolicy, JobKind, Observation,
    ObservedObject, PreparationPolicy, Proposal,
};

fn observation() -> Observation {
    Observation {
        observation_generation: 7,
        objects: vec![
            ObservedObject {
                id: ObjectId("graph.nodes.v1".into()),
                generation: 1,
                logical_size: 100,
                required_authoritative: true,
                locally_present: false,
            },
            ObservedObject {
                id: ObjectId("graph.adjacency.v1".into()),
                generation: 1,
                logical_size: 50,
                required_authoritative: false,
                locally_present: false,
            },
        ],
        budget: Budget::local_default(),
    }
}

#[test]
fn full_transfer_proposes_all_absent_objects() {
    let obs = observation();
    let props = FullTransferPolicy.propose(&obs);
    assert_eq!(props.len(), 2);
    for p in &props {
        assert_eq!(p.job_kind, JobKind::Transfer);
        assert!(
            admit(&obs, p).is_ok(),
            "baseline proposals must be admissible"
        );
    }
}

#[test]
fn demand_defers_everything() {
    let obs = observation();
    let props = DemandPolicy.propose(&obs);
    assert!(props.iter().all(|p| p.job_kind == JobKind::Wait));
    for p in &props {
        assert!(admit(&obs, p).is_ok());
    }
}

#[test]
fn default_transfers_required_defers_rest() {
    let obs = observation();
    let props = DefaultPolicy.propose(&obs);
    let transfers = props
        .iter()
        .filter(|p| p.job_kind == JobKind::Transfer)
        .count();
    let waits = props.iter().filter(|p| p.job_kind == JobKind::Wait).count();
    assert_eq!(
        transfers, 1,
        "only the required authoritative object is transferred eagerly"
    );
    assert_eq!(waits, 1);
}

#[test]
fn core_rejects_inflated_proposal() {
    // A malicious policy claims to move more bytes than the object holds.
    let obs = observation();
    let mut p = FullTransferPolicy.propose(&obs)[0].clone();
    p.estimated_network_bytes = 10_000_000; // object is only 100 bytes
    assert_eq!(
        admit(&obs, &p),
        Err(AdmitRejection::BudgetExceeded(
            "estimate exceeds object size"
        ))
    );
}

#[test]
fn core_rejects_stale_observation() {
    let obs = observation();
    let mut p: Proposal = FullTransferPolicy.propose(&obs)[0].clone();
    p.observation_generation = 999;
    assert_eq!(admit(&obs, &p), Err(AdmitRejection::StaleObservation));
}

#[test]
fn core_rejects_already_present_transfer() {
    let mut obs = observation();
    obs.objects[0].locally_present = true;
    // Craft a transfer proposal for a now-present object.
    let p = Proposal {
        proposal_id: uuid::Uuid::nil(),
        observation_generation: obs.observation_generation,
        job_kind: JobKind::Transfer,
        object_id: obs.objects[0].id.clone(),
        object_version: 1,
        mode: carryon_core::policy::PrepMode::DirectBytes,
        estimated_network_bytes: 10,
        estimated_cpu_millis: 1,
        estimated_peak_memory: 10,
        estimated_nonpreemptible_millis: 1,
        expected_action_benefit: 1.0,
        rationale_code: carryon_core::policy::RationaleCode::AuthoritativeRequired,
    };
    assert_eq!(admit(&obs, &p), Err(AdmitRejection::AlreadyPresent));
}
