//! §26.1 noncanonical representation prefix-consistency; MATH-004.
//!
//! The spec allows multiple full words per source state (§5.4 noncanonical mode),
//! but every valid representation MUST agree on each view-pure prefix. This
//! construction is canonical (one word per state), so we test the invariant two
//! ways:
//!   1. two *different* source states with the *same* coarse view F_t share an
//!      identical length-b_t prefix;
//!   2. the omitted-child choice (FirstChild vs LastChild) is metadata that must
//!      not leak into F_t — both layouts decode the same view for the same state.

mod common;

use carryon_math_core::layout::OmissionPolicy;

#[test]
fn same_view_implies_same_prefix() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    // Two states that differ internally but share F_1 (block {0,1} sum and {2,3}
    // sum). x=[1,2,*,*] and y=[0,3,*,*] both give block{0,1} sum = 3.
    let x = vec![1, 2, 1, 2];
    let y = vec![0, 3, 1, 2]; // same {0,1} sum (3) and identical {2,3} half.
    let wx = enc.encode(&x).unwrap();
    let wy = enc.encode(&y).unwrap();
    let b1 = enc.layout().checkpoint_bound(1);
    assert_eq!(
        wx[..b1],
        wy[..b1],
        "equal F_1 must give equal prefix [0,b_1)"
    );

    // And both decode to the same F_1 view.
    assert_eq!(
        enc.decode_checkpoint(&wx, 1).unwrap(),
        enc.decode_checkpoint(&wy, 1).unwrap()
    );
}

#[test]
fn omission_policy_does_not_change_the_view() {
    let parts = common::h4_balanced();
    let last = common::encoder(5, 4, &parts, OmissionPolicy::LastChild);
    let first = common::encoder(5, 4, &parts, OmissionPolicy::FirstChild);
    let x = vec![1, 2, 3, 4];

    let wl = last.encode(&x).unwrap();
    let wf = first.encode(&x).unwrap();
    // Words may differ (different symbols stored), but every decoded view agrees.
    for t in 0..last.hierarchy().num_levels() {
        assert_eq!(
            last.decode_checkpoint(&wl, t).unwrap().block_sums,
            first.decode_checkpoint(&wf, t).unwrap().block_sums,
            "omitted-child choice must not change F_{t}"
        );
    }
}
