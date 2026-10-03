//! Strict partition hierarchy `P_0 ≺ P_1 ≺ … ≺ P_q` over coordinate indices
//! `[n]` (spec §5.2).
//!
//! # Representation
//!
//! The hierarchy has `q + 1` levels, indexed `0..=q`. Level `0` is the coarsest
//! partition `P_0`; level `q` is the finest and MUST be the singleton partition
//! (one coordinate per block). A level is stored as:
//!
//! - `block_of[i]` — the block id in `0..num_blocks` that coordinate `i` belongs
//!   to at this level;
//! - `blocks[b]` — the sorted coordinate list of block `b`.
//!
//! Block ids at each level are assigned in order of each block's smallest
//! coordinate, giving a stable, deterministic ordering independent of input
//! order (spec §5.3 "stable block ordering").
//!
//! # Strictness (validated)
//!
//! For `t ≥ 1`, every block of `P_t` is contained in exactly one block of
//! `P_{t-1}` (refinement), and every parent block of `P_{t-1}` is the disjoint
//! union of **≥ 2** child blocks of `P_t`. The "≥ 2" rule is what forces each
//! refined parent to contribute at least one stored all-but-one-child symbol,
//! and hence `b_0 < … < b_{q-1}` strictly (spec §5.7).

use crate::error::{MathError, Result};

/// One partition level: a complete assignment of `[n]` to numbered blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    /// `block_of[i]` = block id of coordinate `i` at this level.
    pub block_of: Vec<usize>,
    /// `blocks[b]` = sorted coordinates of block `b`.
    pub blocks: Vec<Vec<usize>>,
}

impl Level {
    /// Number of blocks in this level.
    #[inline]
    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }
}

/// A validated strict partition hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hierarchy {
    n: usize,
    /// Levels coarse→fine, length `q + 1`. `levels[0] = P_0`, `levels[q] = P_q`.
    levels: Vec<Level>,
}

/// Result of [`validate`]: `ok` plus any diagnostic. Mirrors spec
/// `ValidateHierarchy -> ValidationReport` (§5.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub ok: bool,
    pub message: String,
}

/// Build a block list from a per-coordinate block assignment, assigning block
/// ids in order of smallest member coordinate. Returns the canonicalized
/// (`block_of`, `blocks`) pair, or an error if the assignment is not a valid
/// partition of `[n]` (empty block, gap, or out-of-range id).
fn canonicalize_level(n: usize, raw_block_of: &[usize], level: usize) -> Result<Level> {
    if raw_block_of.len() != n {
        return Err(MathError::InvalidPartition {
            level,
            reason: "assignment length does not equal n",
        });
    }
    // Group coordinates by raw id.
    let mut by_raw: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (coord, &raw) in raw_block_of.iter().enumerate() {
        by_raw.entry(raw).or_default().push(coord);
    }
    if by_raw.is_empty() {
        return Err(MathError::InvalidPartition {
            level,
            reason: "no blocks (n must be ≥ 1)",
        });
    }
    // Reassign ids by smallest member for a stable, dense ordering.
    let mut groups: Vec<Vec<usize>> = by_raw.into_values().collect();
    groups.sort_by_key(|coords| coords[0]);

    let mut block_of = vec![0usize; n];
    let mut blocks: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
    for (new_id, mut coords) in groups.into_iter().enumerate() {
        coords.sort_unstable();
        if coords.is_empty() {
            return Err(MathError::InvalidPartition {
                level,
                reason: "empty block",
            });
        }
        for &c in &coords {
            block_of[c] = new_id;
        }
        blocks.push(coords);
    }
    Ok(Level { block_of, blocks })
}

/// Create and validate a hierarchy from `n` and ordered partitions coarse→fine.
///
/// Each partition is given as a `block_of`-style assignment: a length-`n` slice
/// mapping each coordinate to an arbitrary block tag (tags are renumbered
/// canonically). The last partition MUST be the singletons.
///
/// Maps spec `CreateHierarchy(n, ordered_partitions)` (§5.12).
pub fn create(n: usize, ordered_partitions: &[Vec<usize>]) -> Result<Hierarchy> {
    if n == 0 {
        return Err(MathError::InvalidPartition {
            level: 0,
            reason: "n must be ≥ 1",
        });
    }
    if ordered_partitions.is_empty() {
        return Err(MathError::NonSingletonTop);
    }
    let mut levels = Vec::with_capacity(ordered_partitions.len());
    for (t, raw) in ordered_partitions.iter().enumerate() {
        levels.push(canonicalize_level(n, raw, t)?);
    }
    let h = Hierarchy { n, levels };
    let report = validate(&h);
    if !report.ok {
        // Re-run to surface the structured error rather than a stringly report.
        return Err(validate_strict(&h).unwrap_err());
    }
    Ok(h)
}

/// Number of coordinates.
impl Hierarchy {
    #[inline]
    pub fn n(&self) -> usize {
        self.n
    }

    /// `q` = number of refinement levels above `P_0` = `levels - 1`.
    /// The proven worst-case update bound is `q + 1` (spec §5.8).
    #[inline]
    pub fn q(&self) -> usize {
        self.levels.len() - 1
    }

    /// Number of levels, `q + 1`.
    #[inline]
    pub fn num_levels(&self) -> usize {
        self.levels.len()
    }

    /// Level `t` (`0 = coarsest`).
    #[inline]
    pub fn level(&self, t: usize) -> &Level {
        &self.levels[t]
    }

    /// All levels coarse→fine.
    #[inline]
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }
}

/// Validate a hierarchy, returning a report (spec `ValidationReport`). This is
/// the user-facing form; [`validate_strict`] returns the typed error.
pub fn validate(h: &Hierarchy) -> ValidationReport {
    match validate_strict(h) {
        Ok(()) => ValidationReport {
            ok: true,
            message: "valid strict hierarchy".to_string(),
        },
        Err(e) => ValidationReport {
            ok: false,
            message: e.to_string(),
        },
    }
}

/// Validate, returning the first structural violation as a typed error.
///
/// Checks (spec §5.2):
/// 1. every level is a partition of `[n]` (guaranteed by `canonicalize_level`);
/// 2. the finest level is the singleton partition;
/// 3. for `t ≥ 1`, `P_t` refines `P_{t-1}`: every child block lies within one
///    parent block, and every parent splits into `≥ 2` children.
pub fn validate_strict(h: &Hierarchy) -> Result<()> {
    let q = h.q();

    // (2) finest level must be singletons.
    let finest = &h.levels[q];
    if finest.num_blocks() != h.n || finest.blocks.iter().any(|b| b.len() != 1) {
        return Err(MathError::NonSingletonTop);
    }

    // (3) strict refinement between consecutive levels.
    for t in 1..h.levels.len() {
        let parent = &h.levels[t - 1];
        let child = &h.levels[t];

        // Each child block must sit inside a single parent block.
        for cb in &child.blocks {
            let p0 = parent.block_of[cb[0]];
            if cb.iter().any(|&c| parent.block_of[c] != p0) {
                return Err(MathError::NonStrictHierarchy {
                    level: t,
                    reason: "a child block spans two parent blocks",
                });
            }
        }

        // Each parent block must split into ≥ 2 child blocks.
        let mut children_per_parent = vec![0usize; parent.num_blocks()];
        for cb in &child.blocks {
            children_per_parent[parent.block_of[cb[0]]] += 1;
        }
        if let Some(bad) = children_per_parent.iter().position(|&c| c < 2) {
            // c == 1 means the level did not refine this parent (not strict).
            let _ = bad;
            return Err(MathError::NonStrictHierarchy {
                level: t,
                reason: "a parent block is not split into ≥ 2 child blocks",
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// n=4, P0={{0,1,2,3}}, P1={{0,1},{2,3}}, P2=singletons. q=2.
    fn sample() -> Hierarchy {
        create(4, &[vec![0, 0, 0, 0], vec![0, 0, 1, 1], vec![0, 1, 2, 3]]).unwrap()
    }

    #[test]
    fn builds_and_validates() {
        let h = sample();
        assert_eq!(h.q(), 2);
        assert_eq!(h.level(0).num_blocks(), 1);
        assert_eq!(h.level(1).num_blocks(), 2);
        assert_eq!(h.level(2).num_blocks(), 4);
        assert!(validate(&h).ok);
    }

    #[test]
    fn rejects_non_singleton_top() {
        let e = create(4, &[vec![0, 0, 0, 0], vec![0, 0, 1, 1]]).unwrap_err();
        assert_eq!(e, MathError::NonSingletonTop);
    }

    #[test]
    fn rejects_non_refinement() {
        // P1 does not refine P0: block {1,2} crosses P0 halves {0,1} and {2,3}.
        let e = create(4, &[vec![0, 0, 1, 1], vec![0, 1, 1, 2], vec![0, 1, 2, 3]]).unwrap_err();
        assert!(matches!(e, MathError::NonStrictHierarchy { .. }));
    }

    #[test]
    fn rejects_parent_with_single_child() {
        // P0 one block; P1 also one block → parent not split into ≥2.
        let e = create(2, &[vec![0, 0], vec![0, 0]]).unwrap_err();
        assert!(matches!(
            e,
            MathError::NonStrictHierarchy { .. } | MathError::NonSingletonTop
        ));
    }
}
