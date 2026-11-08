//! RESEARCH (stage 11): the block-sum checkpoint index over a REAL sealed cut.
//!
//! Proves the engine hook reads genuine cut rows (`cut_objects` ⨝ `objects`),
//! not caller bytes: register the graph adapter, seal a cut, and encode the
//! authoritative object sizes. Feature-gated exactly like the module.

#![cfg(feature = "blocksum-checkpoint")]

use carryon_adapter_graph::GraphAdapter;
use carryon_core::checkpoint::CheckpointPlan;
use carryon_core::model::{AuthorityMode, Sensitivity};
use carryon_core::{Core, CreateSessionReq};

fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "carryon-blocksum-{tag}-{}",
        carryon_core::ids::SessionId::new()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sealed_cut_coordinates_come_from_the_engine() {
    let dir = temp("cut");
    let mut core = Core::open(&dir).unwrap();
    let info = core
        .register_adapter(Box::new(GraphAdapter::sample()))
        .unwrap();
    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "blocksum".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();

    // The graph adapter seals exactly two authoritative objects.
    let coords = core.cut_checkpoint_coordinates(cut, None).unwrap();
    assert_eq!(coords.len(), 2);
    assert!(coords.iter().all(|&s| s > 0));

    // The prefix filter selects one namespace (here: only the nodes object).
    let nodes = core
        .cut_checkpoint_coordinates(cut, Some("graph.nodes."))
        .unwrap();
    assert_eq!(nodes.len(), 1);

    // Encode the real cut under a 2-cell hierarchy.
    let plan = CheckpointPlan::tile_pyramid(2, 1).unwrap();
    let index = core.checkpoint_index(cut, &plan, None).unwrap();
    assert_eq!(index.source, coords);
    assert_eq!(index.checkpoint_bounds, vec![1, 2]);
    assert!(index.proof_bound_applicable);
    assert!(index.gate_all_pass);
    // View-purity: the coarsest prefix is the total size, at 8 bytes.
    assert_eq!(index.checkpoint_views[0], vec![coords.iter().sum::<u64>()]);

    // The theorem's unit on real engine state.
    let report = core.checkpoint_dilation(cut, &plan, None).unwrap();
    assert!(report.bound_holds);
    assert_eq!(report.upper_bound, 2);
    assert_eq!(report.measured_max, 2);
    assert_eq!(report.coordinates, 2);

    // A plan that does not match the cut's authoritative count fails closed.
    let wrong = CheckpointPlan::tile_pyramid(3, 1).unwrap();
    let err = core.checkpoint_index(cut, &wrong, None).unwrap_err();
    assert!(
        err.to_string().contains("authoritative coordinates"),
        "unexpected error: {err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
