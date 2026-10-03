//! §26.1 encode/decode round trips; MATH-002 (full decode recovers source).

mod common;

use carryon_math_core::layout::OmissionPolicy;

#[test]
fn encode_then_decode_is_identity() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    for x in [
        vec![0, 0, 0, 0],
        vec![1, 2, 3, 4],
        vec![4, 4, 4, 4],
        vec![2, 0, 4, 1],
    ] {
        let word = enc.encode(&x).unwrap();
        assert_eq!(enc.decode_full(&word).unwrap(), x);
    }
}

#[test]
fn word_has_exactly_n_symbols() {
    for policy in [OmissionPolicy::LastChild, OmissionPolicy::FirstChild] {
        let enc = common::encoder(3, 8, &common::h8_deep(), policy);
        let word = enc.encode(&vec![1; 8]).unwrap();
        assert_eq!(word.len(), 8, "construction uses exactly n symbols");
    }
}

#[test]
fn encode_rejects_wrong_length_and_bad_symbols() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    assert!(enc.encode(&vec![1, 2, 3]).is_err());
    assert!(enc.encode(&vec![1, 2, 3, 5]).is_err()); // 5 ∉ Z/5Z
}
