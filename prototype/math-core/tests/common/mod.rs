//! Shared test helpers: naive reference block-sum oracle + hierarchy builders.
//!
//! Level index `t` doubles as both a loop index and a decode argument throughout
//! these tests, so indexing `reference[t]` reads clearer than `enumerate()`.
#![allow(clippy::needless_range_loop)]
// This module is `include`d by every integration test; each test uses only a
// subset of the helpers, so per-test-binary dead-code warnings are expected.
#![allow(dead_code)]

use carryon_math_core::encoder::Encoder;
use carryon_math_core::group::{Elem, Group};
use carryon_math_core::hierarchy::{self, Hierarchy};
use carryon_math_core::layout::OmissionPolicy;

/// Build an encoder from raw parts (panics on invalid input — tests only).
pub fn encoder(k: u64, n: usize, partitions: &[Vec<usize>], policy: OmissionPolicy) -> Encoder {
    let g = Group::new(k).unwrap();
    let h = hierarchy::create(n, partitions).unwrap();
    Encoder::new(g, h, policy)
}

/// A 2-level balanced hierarchy on n=4: {{0..3}} ≺ {{0,1},{2,3}} ≺ singletons. q=2.
pub fn h4_balanced() -> Vec<Vec<usize>> {
    vec![vec![0, 0, 0, 0], vec![0, 0, 1, 1], vec![0, 1, 2, 3]]
}

/// A deeper hierarchy on n=8, q=3: whole ≺ halves ≺ quarters ≺ singletons.
/// Each level strictly refines the previous (every parent splits into 2).
pub fn h8_deep() -> Vec<Vec<usize>> {
    vec![
        vec![0, 0, 0, 0, 0, 0, 0, 0], // P_0: one block {0..7}
        vec![0, 0, 0, 0, 1, 1, 1, 1], // P_1: halves
        vec![0, 0, 1, 1, 2, 2, 3, 3], // P_2: quarters (pairs)
        (0..8).collect(),             // P_3: singletons
    ]
}

/// An unbalanced hierarchy on n=5, q=2: {{0..4}} ≺ {{0,1,2},{3,4}} ≺ singletons.
pub fn h5_unbalanced() -> Vec<Vec<usize>> {
    vec![
        vec![0, 0, 0, 0, 0],
        vec![0, 0, 0, 1, 1],
        vec![0, 1, 2, 3, 4],
    ]
}

/// Naive independent reference: block sums of `x` at every level, straight from
/// the hierarchy definition (no encoder involved). Oracle for MATH-002/003.
pub fn reference_views(h: &Hierarchy, k: u64, x: &[Elem]) -> Vec<Vec<Elem>> {
    (0..h.num_levels())
        .map(|t| {
            let lv = h.level(t);
            lv.blocks
                .iter()
                .map(|coords| coords.iter().map(|&c| x[c]).sum::<u64>() % k)
                .collect::<Vec<Elem>>()
        })
        .collect()
}

/// Enumerate every state in `(Z/kZ)^n` (small n/k only).
pub fn all_states(k: u64, n: usize) -> impl Iterator<Item = Vec<Elem>> {
    let total = (k as u128).pow(n as u32);
    (0..total).map(move |mut code| {
        let mut v = vec![0u64; n];
        for slot in v.iter_mut() {
            *slot = (code % k as u128) as u64;
            code /= k as u128;
        }
        v
    })
}
