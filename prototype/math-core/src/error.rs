//! Error type for the math core.
//!
//! Variants map to the `MATH_*` error family of the engine spec (§24): every
//! failure here is an invalid group, hierarchy, word, delta, prefix, or a
//! proof-applicability failure. Nothing in this crate panics on caller input;
//! malformed input returns an `Err` (spec MATH-001, fail-closed).

use std::fmt;

/// All failures the math core can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MathError {
    /// Group modulus `k < 2` (spec §5.1: `A` must be a nontrivial finite group).
    InvalidGroup { k: u64 },

    /// A level's blocks do not partition `[n]` exactly (overlap, gap, or empty block).
    InvalidPartition { level: usize, reason: &'static str },

    /// The hierarchy is not a strict refinement (spec §5.2): a parent block is not
    /// the disjoint union of **≥2** child blocks, or levels are not ordered coarse→fine.
    NonStrictHierarchy { level: usize, reason: &'static str },

    /// The finest level is not the singleton partition (spec §5.2: `P_q` singleton).
    NonSingletonTop,

    /// A coordinate index is out of range `[0, n)`.
    CoordinateOutOfRange { index: usize, n: usize },

    /// A checkpoint level is out of range `[0, q]`.
    LevelOutOfRange { level: usize, q: usize },

    /// An encoded word has the wrong symbol count (expected exactly `n`).
    WordLengthMismatch { expected: usize, found: usize },

    /// A symbol value is outside the group alphabet `[0, k)`.
    SymbolOutOfRange { index: usize, value: u64, k: u64 },

    /// A theorem-level update delta is zero (spec §5.5: `δ ≠ 0`).
    ZeroDelta,

    /// `VerifyWord` found the word inconsistent with its declared source state.
    InvariantViolation { reason: &'static str },
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use MathError::*;
        match self {
            InvalidGroup { k } => write!(
                f,
                "MATH_GROUP: modulus k={k} < 2 (group must be nontrivial)"
            ),
            InvalidPartition { level, reason } => {
                write!(
                    f,
                    "MATH_HIERARCHY: level {level} is not a valid partition: {reason}"
                )
            }
            NonStrictHierarchy { level, reason } => {
                write!(
                    f,
                    "MATH_HIERARCHY: non-strict refinement at level {level}: {reason}"
                )
            }
            NonSingletonTop => write!(
                f,
                "MATH_HIERARCHY: finest level is not the singleton partition"
            ),
            CoordinateOutOfRange { index, n } => {
                write!(f, "MATH_WORD: coordinate {index} out of range [0, {n})")
            }
            LevelOutOfRange { level, q } => {
                write!(
                    f,
                    "MATH_PREFIX: checkpoint level {level} out of range [0, {q}]"
                )
            }
            WordLengthMismatch { expected, found } => {
                write!(
                    f,
                    "MATH_WORD: word has {found} symbols, expected {expected}"
                )
            }
            SymbolOutOfRange { index, value, k } => {
                write!(
                    f,
                    "MATH_WORD: symbol[{index}]={value} outside group [0, {k})"
                )
            }
            ZeroDelta => write!(f, "MATH_DELTA: theorem-level update requires nonzero delta"),
            InvariantViolation { reason } => write!(f, "MATH_WORD: invariant violation: {reason}"),
        }
    }
}

impl std::error::Error for MathError {}

/// Crate result alias.
pub type Result<T> = std::result::Result<T, MathError>;
