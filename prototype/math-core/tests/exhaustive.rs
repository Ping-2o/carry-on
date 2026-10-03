//! §26.1 exhaustive small-state over multiple finite groups; MATH-002/003/008.
//!
//! For every state in `(Z/kZ)^n` (small n/k), checks: full round trip, every
//! checkpoint equals the reference view, and every single-coordinate nonzero
//! delta changes ≤ q+1 symbols and yields a correct new state.
#![allow(clippy::needless_range_loop)] // `t` is both a loop index and a decode arg

mod common;

use carryon_math_core::hierarchy;
use carryon_math_core::layout::OmissionPolicy;

fn run_case(k: u64, n: usize, parts: &[Vec<usize>], policy: OmissionPolicy) {
    let h = hierarchy::create(n, parts).unwrap();
    let enc = common::encoder(k, n, parts, policy);
    let q = h.q();

    for x in common::all_states(k, n) {
        let word = enc.encode(&x).unwrap();

        // Round trip.
        assert_eq!(enc.decode_full(&word).unwrap(), x);

        // Every checkpoint equals the independent reference.
        let reference = common::reference_views(&h, k, &x);
        for t in 0..h.num_levels() {
            assert_eq!(
                enc.decode_checkpoint(&word, t).unwrap().block_sums,
                reference[t]
            );
        }

        // Verify passes.
        assert!(enc.verify(&word).ok);

        // Every legal single-coordinate update.
        for i in 0..n {
            for d in 1..k {
                let upd = enc.apply_delta(&word, i, d).unwrap();
                assert!(upd.endpoint_changed_symbols <= q + 1, "exceeds q+1 bound");

                // Resulting word decodes to x with x_i bumped by d.
                let mut expected = x.clone();
                expected[i] = (expected[i] + d) % k;
                assert_eq!(enc.decode_full(&upd.updated_word).unwrap(), expected);
            }
        }
    }
}

#[test]
fn exhaustive_z2_z3_z4_z5() {
    for &k in &[2u64, 3, 4, 5] {
        run_case(k, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
        run_case(k, 5, &common::h5_unbalanced(), OmissionPolicy::FirstChild);
    }
    // One deeper hierarchy at the smallest group to keep the state space bounded.
    run_case(2, 8, &common::h8_deep(), OmissionPolicy::LastChild);
}
