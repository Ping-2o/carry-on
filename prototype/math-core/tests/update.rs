//! §26.1 single-coordinate update + exact changed-index verification; MATH-005.

mod common;

use carryon_math_core::encoder::compute_endpoint_distance;
use carryon_math_core::layout::OmissionPolicy;

#[test]
fn changed_indices_match_actual_diff() {
    let enc = common::encoder(7, 8, &common::h8_deep(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![1, 2, 3, 4, 5, 6, 0, 1]).unwrap();

    for i in 0..8 {
        for d in 1..7 {
            let upd = enc.apply_delta(&word, i, d).unwrap();

            // The structurally-reported changed set must equal the real Hamming diff.
            let mut actual: Vec<usize> = (0..word.len())
                .filter(|&r| word[r] != upd.updated_word[r])
                .collect();
            actual.sort_unstable();
            assert_eq!(
                upd.changed_symbol_indices, actual,
                "reported ≠ actual changed set"
            );
            assert_eq!(
                upd.endpoint_changed_symbols,
                compute_endpoint_distance(&word, &upd.updated_word)
            );
        }
    }
}

#[test]
fn rejects_zero_delta_and_bad_index() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![0; 4]).unwrap();
    assert!(
        enc.apply_delta(&word, 0, 0).is_err(),
        "δ=0 must fail (§5.5)"
    );
    assert!(enc.apply_delta(&word, 0, 5).is_err(), "δ≡0 mod k must fail");
    assert!(
        enc.apply_delta(&word, 9, 1).is_err(),
        "bad coordinate must fail"
    );
}

#[test]
fn update_result_carries_all_spec_fields() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![1, 1, 1, 1]).unwrap();
    let upd = enc.apply_delta(&word, 2, 3).unwrap();

    assert_eq!(upd.expected_upper_bound, enc.hierarchy().q() + 1);
    assert!(upd.proof_bound_applicable);
    assert_eq!(
        upd.checkpoint_views_before.len(),
        enc.hierarchy().num_levels()
    );
    assert_eq!(
        upd.checkpoint_views_after.len(),
        enc.hierarchy().num_levels()
    );
    // The coarsest view always changes (coordinate's level-0 block sum moves).
    assert_ne!(
        upd.checkpoint_views_before[0].block_sums,
        upd.checkpoint_views_after[0].block_sums
    );
}
