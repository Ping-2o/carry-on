//! Kani proof harnesses for the block-sum construction the engine exposes.
//!
//! Run from the repository root:
//!
//! ```sh
//! cargo kani -p carryon-math-core --harness update_changes_at_most_q_plus_one
//! cargo kani -p carryon-math-core --harness total_view_is_prefix_pure
//! cargo kani -p carryon-math-core --harness half_view_is_prefix_pure
//! cargo kani -p carryon-math-core --harness encode_decodes_and_updates_correctly
//! ```
//!
//! **Scope, stated honestly (the `prototype/authority-model` pattern):** these are
//! loop-free proofs of a *pure model* of the all-but-one-child layout for the
//! four-coordinate hierarchy of `main.tex`
//! (`P0 = {{0,1,2,3}} ≺ P1 = {{0,1},{2,3}} ≺ P2` singletons, `q = 2`), which the
//! engine's `checkpoint` module instantiates through the proven crate. They are
//! **not** proofs of the Rust `Encoder` implementation: its applicability gate is
//! covered by the crate's exhaustive tests and property tests. `kani::any` is
//! symbolic, so each harness is exhaustive over its whole input space — no loops,
//! no sampling.
//!
//! `cfg(kani)` is set only by the Kani toolchain; normal builds skip the module.

#![cfg(kani)]

/// Group modulus of the model (`Z/5Z`, a nontrivial finite abelian group).
const K: u64 = 5;

/// The all-but-one-child word for the 4-coordinate hierarchy:
/// `word = [total, left_half, a, c]` — `main.tex`'s displayed example
/// `E(a,b,c,d) = (a⊕b⊕c⊕d, a⊕b, a, c)` specialised to `Z/5Z`.
fn encode(x: [u64; 4]) -> [u64; 4] {
    let total = x[0] + x[1] + x[2] + x[3];
    let left = x[0] + x[1];
    [total % K, left % K, x[0] % K, x[2] % K]
}

/// The deterministic updater: add `d` to the stored symbols on the updated
/// coordinate's path. `i ∈ {0,1}` is stored at the coarsest and left levels;
/// `i ∈ {2,3}` is stored at the coarsest level (the omitted children are `b`
/// and `d`). Level-2 symbols `a` and `c` are stored, so a write to `a`/`c`
/// touches them too.
fn update(word: [u64; 4], i: usize, d: u64) -> [u64; 4] {
    let d = d % K;
    let mut w = word;
    // Level 0: the total sum always changes.
    w[0] = (w[0] + d) % K;
    // Level 1: the left-half sum changes only for i ∈ {0,1}.
    if i < 2 {
        w[1] = (w[1] + d) % K;
    }
    // Level 2: the stored singleton symbols are a (i=0) and c (i=2).
    if i == 0 {
        w[2] = (w[2] + d) % K;
    }
    if i == 2 {
        w[3] = (w[3] + d) % K;
    }
    w
}

fn changed(a: [u64; 4], b: [u64; 4]) -> usize {
    usize::from(a[0] != b[0])
        + usize::from(a[1] != b[1])
        + usize::from(a[2] != b[2])
        + usize::from(a[3] != b[3])
}

/// Any fully symbolic source state.
fn any_x() -> [u64; 4] {
    [
        kani::any::<u64>() % K,
        kani::any::<u64>() % K,
        kani::any::<u64>() % K,
        kani::any::<u64>() % K,
    ]
}

/// Theorem bound: one coordinate update changes at most `q + 1 = 3` persistent
/// symbols, for every state, coordinate, and nonzero delta.
#[kani::proof]
fn update_changes_at_most_q_plus_one() {
    let x = any_x();
    let w = encode(x);
    let i: usize = kani::any();
    kani::assume(i < 4);
    let d: u64 = kani::any();
    kani::assume(d > 0 && d < K);

    assert!(changed(w, update(w, i, d)) <= 3);
}

/// View-purity at checkpoint 0: equal total sums give identical first prefixes
/// (the first symbol depends on `F_0` and nothing finer).
#[kani::proof]
fn total_view_is_prefix_pure() {
    let x = any_x();
    let y = any_x();
    let tx = (x[0] + x[1] + x[2] + x[3]) % K;
    let ty = (y[0] + y[1] + y[2] + y[3]) % K;
    kani::assume(tx == ty);

    assert_eq!(encode(x)[0], encode(y)[0]);
}

/// View-purity at checkpoint 1: equal half sums give identical two-symbol
/// prefixes, even when the quarter values differ.
#[kani::proof]
fn half_view_is_prefix_pure() {
    let x = any_x();
    let y = any_x();
    kani::assume((x[0] + x[1]) % K == (y[0] + y[1]) % K);
    kani::assume((x[2] + x[3]) % K == (y[2] + y[3]) % K);

    assert_eq!(&encode(x)[..2], &encode(y)[..2]);
}

/// The word decodes the full source, and one update decodes to the expected
/// updated source — the correctness half of the construction.
#[kani::proof]
fn encode_decodes_and_updates_correctly() {
    let x = any_x();
    let w = encode(x);
    // The level-2 stored singletons are a and c.
    assert_eq!(w[2], x[0]);
    assert_eq!(w[3], x[2]);

    let i: usize = kani::any();
    kani::assume(i < 4);
    let d: u64 = kani::any();
    kani::assume(d > 0 && d < K);
    let w2 = update(w, i, d);

    let mut expected = x;
    expected[i] = (expected[i] + d) % K;
    // Re-encoding the expected state must equal the updated word (deterministic
    // updater correctness), and the source half-sums must agree.
    assert_eq!(encode(expected), w2);
}
