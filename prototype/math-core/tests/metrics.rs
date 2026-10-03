//! §26.1 symbol-vs-bytes/writes/time metric distinction; MATH-006.
//!
//! The theorem cost is `endpoint_changed_symbols` — a Hamming distance over
//! group symbols (§5.6). It is NOT bytes, writes, or time. This test pins the
//! distinction: the reported metric counts changed *symbols*, independent of how
//! many bytes each symbol occupies or how many intermediate writes happened.

mod common;

use carryon_math_core::encoder::compute_endpoint_distance;
use carryon_math_core::layout::OmissionPolicy;

#[test]
fn endpoint_metric_counts_symbols_not_repeated_writes() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![1, 1, 1, 1]).unwrap();
    let upd = enc.apply_delta(&word, 0, 2).unwrap();

    // The metric is exactly the symbol Hamming distance of the two endpoints.
    assert_eq!(
        upd.endpoint_changed_symbols,
        compute_endpoint_distance(&word, &upd.updated_word)
    );
    // Applying the same delta twice to reach the same endpoint via a different
    // path does not change the endpoint distance — it is endpoint-only (§5.6).
    let half = enc.apply_delta(&word, 0, 1).unwrap();
    let full = enc.apply_delta(&half.updated_word, 0, 1).unwrap();
    assert_eq!(
        compute_endpoint_distance(&word, &full.updated_word),
        upd.endpoint_changed_symbols,
        "endpoint distance is path-independent (not a write count)"
    );
}

#[test]
fn zero_distance_when_endpoints_equal() {
    let enc = common::encoder(5, 4, &common::h4_balanced(), OmissionPolicy::LastChild);
    let word = enc.encode(&vec![2, 3, 1, 0]).unwrap();
    // A full cycle of +k on one coordinate returns to the same state: distance 0,
    // even though writes occurred.
    let mut w = word.clone();
    for _ in 0..5 {
        w = enc.apply_delta(&w, 1, 1).unwrap().updated_word;
    }
    assert_eq!(compute_endpoint_distance(&word, &w), 0);
}
