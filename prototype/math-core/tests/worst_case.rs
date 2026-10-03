//! §26.1 worst-case attainment; MATH-008 (construction attains exactly q+1).
//!
//! The lower bound (§5.8) says some legal update must change ≥ q+1 symbols. The
//! all-but-one-child construction attains it: a coordinate that is a *stored*
//! (non-omitted) child at every refinement level changes exactly q+1 symbols —
//! one per level, including level 0.

mod common;

use carryon_math_core::hierarchy;
use carryon_math_core::layout::OmissionPolicy;

/// For coordinate `i`, count levels at which `i`'s block is a stored symbol.
/// The update cost equals that count.
fn cost_for_coord(
    parts: &[Vec<usize>],
    k: u64,
    n: usize,
    i: usize,
    policy: OmissionPolicy,
) -> usize {
    let enc = common::encoder(k, n, parts, policy);
    let word = enc.encode(&vec![0; n]).unwrap();
    enc.apply_delta(&word, i, 1)
        .unwrap()
        .endpoint_changed_symbols
}

#[test]
fn some_coordinate_attains_q_plus_1() {
    for (parts, n) in [
        (common::h4_balanced(), 4usize),
        (common::h8_deep(), 8),
        (common::h5_unbalanced(), 5),
    ] {
        let h = hierarchy::create(n, &parts).unwrap();
        let q = h.q();
        let max_cost = (0..n)
            .map(|i| cost_for_coord(&parts, 5, n, i, OmissionPolicy::LastChild))
            .max()
            .unwrap();
        assert_eq!(max_cost, q + 1, "worst case must equal q+1 (q={q})");
    }
}

#[test]
fn omitted_child_path_is_cheaper() {
    // With LastChild omission, the coordinate(s) in the last child of every parent
    // skip a stored symbol at that level, so cost < q+1 for at least one coord.
    let parts = common::h4_balanced();
    let min_cost = (0..4)
        .map(|i| cost_for_coord(&parts, 5, 4, i, OmissionPolicy::LastChild))
        .min()
        .unwrap();
    let q = hierarchy::create(4, &parts).unwrap().q();
    assert!(
        min_cost < q + 1,
        "omitted-child coordinate should cost < q+1"
    );
}
