//! §26.1 fuzzing of malformed words and hierarchies; MATH-001 (fail-closed, no panic).

mod common;

use carryon_math_core::hierarchy;
use carryon_math_core::layout::OmissionPolicy;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    /// Arbitrary byte-words must be rejected or decoded without panicking.
    #[test]
    fn malformed_words_never_panic(raw in prop::collection::vec(0u64..1000, 0..12)) {
        let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
        // Any of these may Err; none may panic.
        let _ = enc.verify(&raw);
        let _ = enc.decode_full(&raw);
        let _ = enc.decode_checkpoint(&raw, 0);
        let _ = enc.apply_delta(&raw, 0, 1);
        prop_assert!(true);
    }

    /// Arbitrary partition assignments either build a valid hierarchy or Err.
    #[test]
    fn malformed_hierarchies_never_panic(
        a in prop::collection::vec(0usize..4, 4),
        b in prop::collection::vec(0usize..4, 4),
    ) {
        // Finest level forced to singletons; middle levels arbitrary.
        let res = hierarchy::create(4, &[a, b, vec![0, 1, 2, 3]]);
        match res {
            Ok(h) => prop_assert!(hierarchy::validate(&h).ok),
            Err(_) => prop_assert!(true),
        }
    }
}

#[test]
fn wrong_length_word_is_rejected_not_panicked() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    assert!(enc.decode_full(&vec![1, 2]).is_err());
    assert!(!enc.verify(&vec![1, 2, 3, 4, 5]).ok);
}
