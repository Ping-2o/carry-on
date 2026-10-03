//! `AllButOneChildEncoder` — the matching construction of spec §5.9.
//!
//! Stores exactly `n` group symbols and has worst-case single-coordinate update
//! cost exactly `q + 1` (spec §5.8 lower bound, §5.9 attainment). See
//! [`crate::layout`] for the symbol layout and the view-pure prefix argument.

use crate::error::{MathError, Result};
use crate::group::{Elem, Group};
use crate::hierarchy::Hierarchy;
use crate::layout::{Layout, OmissionPolicy};

/// Prefix-semantics label for the encoder manifest (spec §5.10).
pub const PREFIX_SEMANTICS: &str = "view-pure";
/// Proof-assumption version tag (spec §5.10).
pub const PROOF_ASSUMPTION_VERSION: &str = "vp-1";

/// An encoded persistent word: exactly `n` group symbols (spec §5.4).
pub type Word = Vec<Elem>;

/// A source state vector `x ∈ A^n` (spec §5.1).
pub type SourceVector = Vec<Elem>;

/// A block-sum view `F_t(x)`: `block_sums[b]` is the sum of block `b` at the
/// decoded level, in block order (spec §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSumView {
    pub level: usize,
    pub block_sums: Vec<Elem>,
}

/// Result of a single-coordinate delta update (spec §5.12 `UpdateResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateResult {
    pub updated_word: Word,
    /// Exact set of symbol indices whose value changed (sorted, deduped).
    pub changed_symbol_indices: Vec<usize>,
    /// `endpoint_changed_symbols` = Hamming distance of the words (spec §5.6).
    pub endpoint_changed_symbols: usize,
    /// The proven worst-case upper bound `q + 1` (spec §5.8).
    pub expected_upper_bound: usize,
    /// Whether the `q + 1` bound applies to this encoder (true for view-pure).
    pub proof_bound_applicable: bool,
    /// `F_0..F_q` before the update.
    pub checkpoint_views_before: Vec<BlockSumView>,
    /// `F_0..F_q` after the update.
    pub checkpoint_views_after: Vec<BlockSumView>,
}

/// Outcome of [`Encoder::verify`] (spec `VerificationReport`, §5.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    pub ok: bool,
    pub message: String,
}

/// The encoder handle: a validated group + hierarchy + symbol layout.
#[derive(Debug, Clone)]
pub struct Encoder {
    group: Group,
    hierarchy: Hierarchy,
    layout: Layout,
}

impl Encoder {
    /// Create an all-but-one-child encoder (spec `CreateAllButOneEncoder`).
    pub fn new(group: Group, hierarchy: Hierarchy, policy: OmissionPolicy) -> Encoder {
        let layout = Layout::build(&hierarchy, &policy);
        Encoder {
            group,
            hierarchy,
            layout,
        }
    }

    // --- read-only manifest flags (spec §5.10, MATH-007) ---

    pub fn prefix_semantics(&self) -> &'static str {
        PREFIX_SEMANTICS
    }
    pub fn proof_bound_applicable(&self) -> bool {
        true
    }
    pub fn proof_assumption_version(&self) -> &'static str {
        PROOF_ASSUMPTION_VERSION
    }

    pub fn group(&self) -> &Group {
        &self.group
    }
    pub fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Number of source coordinates / symbols.
    #[inline]
    pub fn n(&self) -> usize {
        self.hierarchy.n()
    }

    /// Worst-case update bound `q + 1` (spec §5.8).
    #[inline]
    pub fn worst_case_bound(&self) -> usize {
        self.hierarchy.q() + 1
    }

    /// Raw block sums of a source vector at one level (helper; not a view object).
    fn block_sums_at(&self, x: &[Elem], level: usize) -> Vec<Elem> {
        let lv = self.hierarchy.level(level);
        let mut sums = vec![self.group.zero(); lv.num_blocks()];
        for (b, coords) in lv.blocks.iter().enumerate() {
            let mut acc = self.group.zero();
            for &c in coords {
                acc = self.group.add(acc, x[c]);
            }
            sums[b] = acc;
        }
        sums
    }

    /// Encode a source vector into its persistent word (spec `Encode`).
    ///
    /// The word is laid out by [`Layout`]: level-0 block sums, then all-but-one
    /// child sums per refinement level.
    pub fn encode(&self, x: &SourceVector) -> Result<Word> {
        if x.len() != self.n() {
            return Err(MathError::CoordinateOutOfRange {
                index: x.len(),
                n: self.n(),
            });
        }
        for (i, &v) in x.iter().enumerate() {
            if !self.group.in_alphabet(v) {
                return Err(MathError::SymbolOutOfRange {
                    index: i,
                    value: v,
                    k: self.group.modulus(),
                });
            }
        }
        let mut word = vec![self.group.zero(); self.n()];
        // Precompute per-level block sums, then scatter stored ones into slots.
        for t in 0..self.hierarchy.num_levels() {
            let sums = self.block_sums_at(x, t);
            for (b, &s) in sums.iter().enumerate() {
                if let Some(slot) = self.layout.symbol_of(t, b) {
                    word[slot] = s;
                }
            }
        }
        Ok(word)
    }

    /// Validate a word's shape and alphabet (not its consistency with a source).
    fn check_word_shape(&self, word: &Word) -> Result<()> {
        if word.len() != self.n() {
            return Err(MathError::WordLengthMismatch {
                expected: self.n(),
                found: word.len(),
            });
        }
        for (i, &v) in word.iter().enumerate() {
            if !self.group.in_alphabet(v) {
                return Err(MathError::SymbolOutOfRange {
                    index: i,
                    value: v,
                    k: self.group.modulus(),
                });
            }
        }
        Ok(())
    }

    /// Decode the block-sum view `F_t` from the prefix `[0, b_t)` of a word
    /// (spec `DecodeCheckpoint`). Only the first `b_t` symbols are read.
    pub fn decode_checkpoint(&self, word: &Word, level: usize) -> Result<BlockSumView> {
        self.check_word_shape(word)?;
        let q = self.hierarchy.q();
        if level > q {
            return Err(MathError::LevelOutOfRange { level, q });
        }
        // Reconstruct block sums level by level down to `level`, folding each
        // parent sum into its children (omitted child = parent − Σ siblings).
        // `cur` holds the fully-known block sums of the current level.
        let mut cur = self.recover_level_sums(word, 0)?;
        for t in 1..=level {
            cur = self.refine_sums(word, t, &cur)?;
        }
        Ok(BlockSumView {
            level,
            block_sums: cur,
        })
    }

    /// Block sums at level 0 are stored verbatim.
    fn recover_level_sums(&self, word: &Word, level: usize) -> Result<Vec<Elem>> {
        debug_assert_eq!(level, 0);
        let lv = self.hierarchy.level(0);
        let mut sums = vec![self.group.zero(); lv.num_blocks()];
        for (b, s) in sums.iter_mut().enumerate() {
            let slot = self
                .layout
                .symbol_of(0, b)
                .expect("every level-0 block is stored");
            *s = word[slot];
        }
        Ok(sums)
    }

    /// Given the known block sums of level `t-1` (`parent_sums`), reconstruct the
    /// block sums of level `t` using stored children and omitted-child recovery.
    fn refine_sums(&self, word: &Word, t: usize, parent_sums: &[Elem]) -> Result<Vec<Elem>> {
        let child = self.hierarchy.level(t);
        let mut sums = vec![self.group.zero(); child.num_blocks()];

        for (p, &parent_sum) in parent_sums.iter().enumerate() {
            let (children, omitted_pos) = self.layout.parent_children(t, p);
            // Sum stored children; omitted = parent_sum − that sum.
            let mut stored_total = self.group.zero();
            for (pos, &cb) in children.iter().enumerate() {
                if pos == omitted_pos {
                    continue;
                }
                let slot = self
                    .layout
                    .symbol_of(t, cb)
                    .expect("non-omitted child is stored");
                sums[cb] = word[slot];
                stored_total = self.group.add(stored_total, word[slot]);
            }
            let omitted_cb = children[omitted_pos];
            sums[omitted_cb] = self.group.sub(parent_sum, stored_total);
        }
        Ok(sums)
    }

    /// Decode the full source vector (spec `DecodeFull`). Equivalent to the
    /// checkpoint at the finest level, whose singleton blocks are the `x_i`.
    pub fn decode_full(&self, word: &Word) -> Result<SourceVector> {
        let q = self.hierarchy.q();
        let view = self.decode_checkpoint(word, q)?;
        let finest = self.hierarchy.level(q);
        // Map each singleton block's sum back to its coordinate.
        let mut x = vec![self.group.zero(); self.n()];
        for (b, coords) in finest.blocks.iter().enumerate() {
            debug_assert_eq!(coords.len(), 1);
            x[coords[0]] = view.block_sums[b];
        }
        Ok(x)
    }

    /// All checkpoint views `F_0..=F_q` for a word.
    pub fn all_checkpoint_views(&self, word: &Word) -> Result<Vec<BlockSumView>> {
        (0..self.hierarchy.num_levels())
            .map(|t| self.decode_checkpoint(word, t))
            .collect()
    }

    /// Apply a single-coordinate delta `x_i ← x_i + δ` to the word (spec
    /// `ApplyCoordinateDelta`, §5.5). Returns the updated word and the exact set
    /// of changed symbols; `endpoint_changed_symbols ≤ q + 1` always holds.
    pub fn apply_delta(&self, word: &Word, i: usize, delta: Elem) -> Result<UpdateResult> {
        self.check_word_shape(word)?;
        if i >= self.n() {
            return Err(MathError::CoordinateOutOfRange {
                index: i,
                n: self.n(),
            });
        }
        let d = self.group.canon(delta);
        if d == 0 {
            return Err(MathError::ZeroDelta);
        }

        let before_views = self.all_checkpoint_views(word)?;

        // Coordinate i sits in one block per level. A stored block's symbol gains
        // δ; an omitted child's "symbol" does not exist, so nothing is written
        // for it — this is where the ≤ q+1 saving comes from.
        let mut updated = word.clone();
        let mut changed: Vec<usize> = Vec::with_capacity(self.hierarchy.num_levels());
        for t in 0..self.hierarchy.num_levels() {
            let block = self.hierarchy.level(t).block_of[i];
            if let Some(slot) = self.layout.symbol_of(t, block) {
                updated[slot] = self.group.add(updated[slot], d);
                changed.push(slot);
            }
        }
        changed.sort_unstable();
        changed.dedup();

        let after_views = self.all_checkpoint_views(&updated)?;

        Ok(UpdateResult {
            updated_word: updated,
            endpoint_changed_symbols: changed.len(),
            changed_symbol_indices: changed,
            expected_upper_bound: self.worst_case_bound(),
            proof_bound_applicable: true,
            checkpoint_views_before: before_views,
            checkpoint_views_after: after_views,
        })
    }

    /// Verify that a word is internally consistent: decoding the full state and
    /// re-encoding reproduces the word exactly (spec `VerifyWord`). Catches any
    /// word that is not a valid encoding of some source state.
    pub fn verify(&self, word: &Word) -> VerificationReport {
        if let Err(e) = self.check_word_shape(word) {
            return VerificationReport {
                ok: false,
                message: e.to_string(),
            };
        }
        match self.decode_full(word).and_then(|x| self.encode(&x)) {
            Ok(re) if &re == word => VerificationReport {
                ok: true,
                message: "word is a valid encoding".to_string(),
            },
            Ok(_) => VerificationReport {
                ok: false,
                message: MathError::InvariantViolation {
                    reason: "re-encode of decoded state differs from word",
                }
                .to_string(),
            },
            Err(e) => VerificationReport {
                ok: false,
                message: e.to_string(),
            },
        }
    }
}

/// Hamming distance over two equal-length symbol words (spec
/// `ComputeEndpointDistance`, §5.6). Counts endpoint symbol differences only.
pub fn compute_endpoint_distance(a: &[Elem], b: &[Elem]) -> usize {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
}
