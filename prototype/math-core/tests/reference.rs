//! §26.1 independent reference implementation comparison; MATH-002.
//!
//! A naive, from-scratch encoder/decoder (no shared code with the crate's
//! construction) cross-checks the real one. The reference stores *all* child
//! sums (not all-but-one) — a simpler, obviously-correct layout — so agreement
//! on decoded views is strong evidence the optimized construction is correct.
//! Also exercises deterministic test-vector export.
#![allow(clippy::needless_range_loop)] // `t` is both a loop index and a decode arg

mod common;

use carryon_math_core::api::export_test_vectors;
use carryon_math_core::hierarchy;
use carryon_math_core::layout::OmissionPolicy;

#[test]
fn crate_views_match_naive_reference() {
    let parts = common::h5_unbalanced();
    let h = hierarchy::create(5, &parts).unwrap();

    for &k in &[2u64, 3, 6] {
        let enc = common::encoder(k, 5, &parts, OmissionPolicy::LastChild);
        for x in common::all_states(k, 5).take(3000) {
            let word = enc.encode(&x).unwrap();
            let reference = common::reference_views(&h, k, &x); // naive oracle
            for t in 0..h.num_levels() {
                assert_eq!(
                    enc.decode_checkpoint(&word, t).unwrap().block_sums,
                    reference[t]
                );
            }
        }
    }
}

#[test]
fn test_vector_export_is_deterministic() {
    let enc = common::encoder(7, 8, &common::h8_deep(), OmissionPolicy::LastChild);
    let a = export_test_vectors(&enc, 0xC0FFEE, 32).unwrap();
    let b = export_test_vectors(&enc, 0xC0FFEE, 32).unwrap();
    assert_eq!(a, b, "same seed must yield byte-identical vectors");

    let c = export_test_vectors(&enc, 0xBEEF, 32).unwrap();
    assert_ne!(a, c, "different seed should differ");

    // Every exported vector is self-consistent.
    for v in &a.vectors {
        assert_eq!(enc.encode(&v.source).unwrap(), v.word);
        assert!(v.update.endpoint_changed_symbols <= enc.hierarchy().q() + 1);
    }
}
