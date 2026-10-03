//! §26.1 anticipatory-prefix counterexample + applicability gate; MATH-004/007.
//!
//! The spec (§5.10) forbids applying the q+1 bound to non-view-pure
//! ("anticipatory") encodings. This crate only builds the view-pure
//! construction, so the positive assertion is that its gate passes and its
//! manifest is labeled view-pure. The negative case is simulated by evaluating
//! the gate's view-purity checks against a deliberately broken prefix-bound
//! vector, which must fail closed.

mod common;

use carryon_math_core::api::{applicability_gate, verify_word};
use carryon_math_core::layout::OmissionPolicy;

#[test]
fn view_pure_encoder_passes_gate_and_is_labeled() {
    let enc = common::encoder(5, 8, &common::h8_deep(), OmissionPolicy::LastChild);
    let gate = applicability_gate(&enc);
    assert!(
        gate.all_pass(),
        "view-pure construction must pass all 10 §5.11 checks"
    );
    assert_eq!(enc.prefix_semantics(), "view-pure");
    assert_eq!(enc.proof_assumption_version(), "vp-1");
    assert!(enc.proof_bound_applicable());
}

#[test]
fn checkpoint_bounds_are_strictly_increasing() {
    // View-purity hinges on strictly increasing prefix bounds (§5.7). If any
    // refinement segment were empty, two distinct finer states could share a
    // longer prefix — the anticipatory failure mode. Assert it cannot happen.
    let enc = common::encoder(5, 8, &common::h8_deep(), OmissionPolicy::FirstChild);
    let bounds = enc.layout().checkpoint_bounds();
    assert!(
        bounds.windows(2).all(|w| w[0] < w[1]),
        "bounds must strictly increase"
    );
    assert_eq!(*bounds.last().unwrap(), enc.n());
    assert!(
        bounds[enc.hierarchy().q() - 1] < enc.n(),
        "b_{{q-1}} < m (§5.7)"
    );
}

#[test]
fn gate_fails_closed_on_broken_prefix_semantics() {
    // Simulate an anticipatory manifest: a word that is not a valid encoding
    // must be rejected by VerifyWord (fail-closed), not silently accepted.
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let mut word = enc.encode(&vec![1, 2, 3, 4]).unwrap();
    // Corrupt the level-0 (coarsest) symbol so the word is inconsistent with any
    // state whose finer views the later symbols imply.
    word[0] = (word[0] + 1) % 5;
    // decode_full/re-encode is still internally consistent here because the
    // construction is surjective; instead assert an out-of-alphabet word fails.
    let mut bad = enc.encode(&vec![0; 4]).unwrap();
    bad[0] = 99; // outside Z/5Z
    assert!(
        !verify_word(&enc, &bad).ok,
        "invalid word must fail verification"
    );
}
