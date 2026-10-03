//! §26.1 checkpoint decode; MATH-003 (checkpoint recovers declared block-sum view).
#![allow(clippy::needless_range_loop)] // `t` is both a loop index and a decode arg

mod common;

use carryon_math_core::hierarchy;
use carryon_math_core::layout::OmissionPolicy;

#[test]
fn checkpoint_matches_reference_view_every_level() {
    let parts = common::h8_deep();
    let h = hierarchy::create(8, &parts).unwrap();
    let enc = common::encoder(4, 8, &parts, OmissionPolicy::LastChild);

    for x in [vec![1, 2, 3, 0, 1, 2, 3, 0], vec![3, 3, 3, 3, 0, 0, 0, 0]] {
        let word = enc.encode(&x).unwrap();
        let reference = common::reference_views(&h, 4, &x);
        for t in 0..h.num_levels() {
            let view = enc.decode_checkpoint(&word, t).unwrap();
            assert_eq!(view.block_sums, reference[t], "F_{t} mismatch");
        }
    }
}

#[test]
fn checkpoint_reads_only_the_prefix() {
    // Corrupting a symbol strictly after b_t must NOT change the decoded F_t.
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![1, 2, 3, 4]).unwrap();
    let b1 = enc.layout().checkpoint_bound(1);

    let view_before = enc.decode_checkpoint(&word, 1).unwrap();
    let mut tampered = word.clone();
    // Flip a symbol at or after b_1 (exists because b_1 < n).
    tampered[b1] = (tampered[b1] + 1) % 5;
    let view_after = enc.decode_checkpoint(&tampered, 1).unwrap();

    assert_eq!(
        view_before, view_after,
        "F_1 must depend only on prefix [0, b_1)"
    );
}

#[test]
fn rejects_level_out_of_range() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![0; 4]).unwrap();
    assert!(enc.decode_checkpoint(&word, 99).is_err());
}
