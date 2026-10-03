//! Math-view bridge (§4.3/§5.10/§23.2): block-sum checkpoints as a progressive
//! view, the §5.11 gate passes, and a single-counter update stays within q+1.

use carryon_core::mathview::{encode_view, update_metrics};

fn partitions() -> Vec<Vec<usize>> {
    // n=4: {{0..3}} ≺ {{0,1},{2,3}} ≺ singletons  (q=2).
    vec![vec![0, 0, 0, 0], vec![0, 0, 1, 1], vec![0, 1, 2, 3]]
}

#[test]
fn progressive_view_is_labeled_and_gated() {
    let counters = vec![1, 2, 3, 4];
    let view = encode_view(10, &counters, &partitions()).unwrap();
    assert_eq!(view.prefix_semantics, "view-pure");
    assert_eq!(view.proof_assumption_version, "vp-1");
    assert!(view.proof_bound_applicable);
    assert!(view.gate_all_pass, "§5.11 applicability gate must pass");
    assert_eq!(view.hierarchy_depth, 2);
    // Coarsest view F_0 = total sum mod 10 = (1+2+3+4) % 10 = 0.
    assert_eq!(view.checkpoint_views[0], vec![0]);
    // Checkpoint bounds strictly increase, last == n.
    let b = &view.checkpoint_bounds;
    assert!(b.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(*b.last().unwrap(), 4);
}

#[test]
fn single_counter_update_respects_proven_bound() {
    let counters = vec![1, 2, 3, 4];
    let m = update_metrics(10, &counters, &partitions(), 0, 1).unwrap();
    assert_eq!(m.expected_upper_bound, 3); // q+1 = 3
    assert!(
        m.endpoint_changed_symbols <= m.expected_upper_bound,
        "update must change at most q+1 symbols (spec §5.8)"
    );
    assert!(m.proof_bound_applicable);
    assert_eq!(m.changed_symbol_indices.len(), m.endpoint_changed_symbols);
}
