//! Deterministic test-vector export (spec `ExportTestVectors`, §5.9/§5.12).
//!
//! Uses a hand-rolled splitmix64 PRNG so vectors are byte-reproducible for a
//! fixed seed with **zero** runtime dependencies. Each vector is a source state,
//! its encoded word, a single legal coordinate delta, and the resulting update
//! result — enough for an independent implementation to cross-check.

use crate::encoder::{Encoder, UpdateResult, Word};
use crate::error::Result;
use crate::group::Elem;

/// splitmix64: a tiny, well-known, fully deterministic PRNG.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish value in `[0, bound)` for small bounds (bias negligible here).
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

/// One exported test vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestVector {
    pub source: Vec<Elem>,
    pub word: Word,
    pub delta_index: usize,
    pub delta_value: Elem,
    pub update: UpdateResult,
}

/// A deterministic bundle of vectors for one encoder + seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestVectorBundle {
    pub seed: u64,
    pub n: usize,
    pub k: u64,
    pub q: usize,
    pub vectors: Vec<TestVector>,
}

/// Export `count` deterministic test vectors for `encoder` from `seed`
/// (spec `ExportTestVectors(encoder, seed, count)`).
pub fn export(encoder: &Encoder, seed: u64, count: usize) -> Result<TestVectorBundle> {
    let k = encoder.group().modulus();
    let n = encoder.n();
    let mut rng = SplitMix64::new(seed);
    let mut vectors = Vec::with_capacity(count);

    for _ in 0..count {
        let source: Vec<Elem> = (0..n).map(|_| rng.below(k)).collect();
        let word = encoder.encode(&source)?;

        // Pick a coordinate and a nonzero delta.
        let idx = if n == 0 {
            0
        } else {
            rng.below(n as u64) as usize
        };
        let mut dv = rng.below(k);
        if dv == 0 {
            dv = 1; // ensure nonzero (k ≥ 2 guarantees 1 is valid and nonzero)
        }
        let update = encoder.apply_delta(&word, idx, dv)?;

        vectors.push(TestVector {
            source,
            word,
            delta_index: idx,
            delta_value: dv,
            update,
        });
    }

    Ok(TestVectorBundle {
        seed,
        n,
        k,
        q: encoder.hierarchy().q(),
        vectors,
    })
}
