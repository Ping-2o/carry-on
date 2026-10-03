//! §26.1 random property tests; MATH-005/008 via proptest.
//!
//! Random states and deltas over random moduli, on fixed valid hierarchies, must
//! satisfy: round trip, every checkpoint = reference view, update ≤ q+1, update
//! correctness, and the §5.11 gate passes.
#![allow(clippy::needless_range_loop)] // `t` is both a loop index and a decode arg

mod common;

use carryon_math_core::api::applicability_gate;
use carryon_math_core::hierarchy;
use carryon_math_core::layout::OmissionPolicy;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn roundtrip_and_update_invariants(
        k in 2u64..9,
        xs in prop::collection::vec(0u64..64, 8),
        i in 0usize..8,
        draw_d in 1u64..64,
    ) {
        let parts = common::h8_deep();
        let h = hierarchy::create(8, &parts).unwrap();
        let enc = common::encoder(k, 8, &parts, OmissionPolicy::LastChild);
        let q = h.q();

        // Reduce the drawn state into Z/kZ.
        let x: Vec<u64> = xs.iter().map(|v| v % k).collect();
        let word = enc.encode(&x).unwrap();

        // Round trip.
        prop_assert_eq!(enc.decode_full(&word).unwrap(), x.clone());

        // Checkpoints equal reference.
        let reference = common::reference_views(&h, k, &x);
        for t in 0..h.num_levels() {
            prop_assert_eq!(enc.decode_checkpoint(&word, t).unwrap().block_sums, reference[t].clone());
        }

        // Gate.
        prop_assert!(applicability_gate(&enc).all_pass());

        // Update with a nonzero delta.
        let d = {
            let dd = draw_d % k;
            if dd == 0 { 1 } else { dd }
        };
        let upd = enc.apply_delta(&word, i, d).unwrap();
        prop_assert!(upd.endpoint_changed_symbols <= q + 1);

        let mut expected = x.clone();
        expected[i] = (expected[i] + d) % k;
        prop_assert_eq!(enc.decode_full(&upd.updated_word).unwrap(), expected);
    }
}
