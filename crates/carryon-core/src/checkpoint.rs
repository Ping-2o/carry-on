//! View-pure block-sum checkpoint index over a **sealed cut** (RESEARCH, stage 11).
//!
//! This module makes the `main.tex` block-sum theorem load-bearing in the engine:
//! a sealed cut's authoritative object sizes are modelled as a source vector
//! `x ∈ (Z/kZ)^n` under a strict partition hierarchy derived from the adapter's
//! **tile grid**. The persistent representation is the all-but-one-child
//! construction of `carryon-math-core` (§5.9), whose proved worst-case
//! single-coordinate update cost is exactly `q + 1` (§5.8) *while every
//! checkpoint prefix stays view-pure* (§5.7).
//!
//! # Encoding site: why the sealed cut, not the journal or the store
//!
//! * The **journal** is append-only and hash-chained: an event append never
//!   rewrites earlier symbols, so endpoint Hamming distance is the wrong cost
//!   model there.
//! * The **store layout** is content-addressed: an object's path is a digest, so
//!   the first `b_t` bytes of a stored object are payload bytes, not an exact
//!   block-sum view. A file prefix is not a checkpoint under this contract.
//! * The **sealed cut** is the engine's immutable statement of a handoff boundary
//!   (§6.2) and is defined by an ordered vector of authoritative object
//!   versions. Its coordinate `i` is the served size of authoritative object `i`
//!   (`logical_size`); one tile whose served size changes by a byte is one
//!   coordinate update. A checkpoint prefix then decodes the exact total bytes of
//!   a coarse region (whole document → row bands → rows → half-rows → tiles),
//!   which is precisely the progressive view the engine exposes for inspection.
//!   The view is a **size summary**, not a content digest: a tile edit that
//!   preserves the served byte length does not move its coordinate.
//!
//! The representation is **not on the production path**. `blocksum-checkpoint`
//! is an opt-in core feature; `carryon-ffi` (and therefore both apps) never
//! enables it. Nothing is persisted: the index is a *derived* view of a sealed
//! cut, and the update metric is the endpoint Hamming distance between the words
//! of two states (the theorem's own metric — not bytes written, not wire bytes).
//!
//! Honest boundary: the theorem bounds one source-coordinate update. A cut that
//! changes several tiles at once is a multi-coordinate update and has no `q + 1`
//! bound; [`CheckpointPlan::endpoint_distance`] reports it without claiming the
//! theorem. Delivery bytes are not the theorem metric (see `bench-blocksum`).

use crate::core::Core;
use crate::error::{CoreError, InternalCode, Result, SchemaCode};
use crate::ids::CutId;
use carryon_math_core::api;
use carryon_math_core::layout::OmissionPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Default group modulus for size views. Real object sizes are far below it, so
/// block sums do not wrap (the view stays an exact byte count). The theorem
/// itself only needs a nontrivial finite abelian group.
pub const DEFAULT_GROUP_MODULUS: u64 = 1 << 32;

/// A strict partition hierarchy over a rectangular tile grid plus the group.
///
/// `partitions` are `block_of` assignments, coarse → fine (the same form
/// `carryon_math_core::api::create_hierarchy` takes); the finest is the
/// singleton partition. The plan is pure data: build it once, encode any
/// coordinate vector of the same length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPlan {
    /// Group modulus `k` of `Z/kZ` (must be `>= 2`).
    pub group_modulus: u64,
    /// Number of coordinates `n = cols * rows`.
    pub coordinates: usize,
    pub cols: usize,
    pub rows: usize,
    /// `partitions[t][i]` = block id of coordinate `i` at level `t`.
    pub partitions: Vec<Vec<usize>>,
}

/// The derived checkpoint index of one cut state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointIndex {
    pub group_modulus: u64,
    pub coordinates: usize,
    /// `q` = hierarchy depth (`levels - 1`).
    pub hierarchy_depth: usize,
    /// Proved worst-case bound `q + 1` for one coordinate update.
    pub expected_upper_bound: usize,
    /// Checkpoint prefix lengths `b_0 < … < b_q` (`b_q == n`).
    pub checkpoint_bounds: Vec<usize>,
    /// Decoded views `F_0 … F_q` (each read from its own prefix only).
    pub checkpoint_views: Vec<Vec<u64>>,
    /// The persistent word (the all-but-one-child construction).
    pub word: Vec<u64>,
    /// The source vector the word encodes (object sizes, row-major).
    pub source: Vec<u64>,
    /// Manifest labels carried from the proven construction (§5.10).
    pub prefix_semantics: String,
    pub proof_assumption_version: String,
    pub proof_bound_applicable: bool,
    /// §5.11 applicability gate (MATH-007).
    pub gate_all_pass: bool,
}

/// One single-coordinate update measured over a real word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingleUpdate {
    pub index: usize,
    pub delta: u64,
    /// Symbols whose final values differ (the theorem's endpoint metric).
    pub changed_symbols: usize,
    pub changed_symbol_indices: Vec<usize>,
    /// `q + 1`.
    pub upper_bound: usize,
    pub bound_holds: bool,
    /// For each checkpoint `t`: changed symbols **inside** prefix `[0, b_t)`.
    /// The compatible-layer argument gives `<= t + 1` (main.tex eq. 13).
    pub prefix_changed: Vec<usize>,
    pub prefix_change_bound_holds: bool,
}

/// Aggregate endpoint dilation over every single-coordinate update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DilationReport {
    pub coordinates: usize,
    pub hierarchy_depth: usize,
    pub upper_bound: usize,
    pub measured_max: usize,
    pub measured_min: usize,
    /// Mean × 1000 (integer, so the report is exactly reproducible).
    pub measured_mean_milli: usize,
    pub attaining_indices: Vec<usize>,
    pub histogram: BTreeMap<usize, usize>,
    pub bound_holds: bool,
    /// Checkpoint delivery sizes in bytes if each symbol is stored as `u64`
    /// (`b_t * 8`) — a metadata view, NOT a document-delivery saving.
    pub prefix_bytes_u64: Vec<usize>,
}

impl CheckpointPlan {
    /// A pyramid over a `cols × rows` tile grid. Levels, coarse → fine:
    /// whole grid → two row bands → rows → two column halves per row →
    /// cells (singletons). Candidates that would not strictly refine the
    /// previous level are skipped, so the plan is valid for any `cols, rows >= 1`
    /// (including a single cell, which yields `q = 0`).
    pub fn tile_pyramid(cols: usize, rows: usize) -> Result<Self> {
        Self::tile_pyramid_with_modulus(DEFAULT_GROUP_MODULUS, cols, rows)
    }

    /// [`Self::tile_pyramid`] with an explicit group modulus.
    pub fn tile_pyramid_with_modulus(k: u64, cols: usize, rows: usize) -> Result<Self> {
        if cols == 0 || rows == 0 {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                "checkpoint grid needs cols >= 1 and rows >= 1",
            ));
        }
        let n = cols.checked_mul(rows).ok_or_else(|| {
            CoreError::schema(SchemaCode::Invalid, "checkpoint grid overflows usize")
        })?;
        let partitions = pyramid_partitions(cols, rows);
        let plan = CheckpointPlan {
            group_modulus: k,
            coordinates: n,
            cols,
            rows,
            partitions,
        };
        plan.validate()?;
        Ok(plan)
    }

    /// Validate the plan by asking the proven prototype to build the hierarchy
    /// (it rejects non-partitions, non-strict refinements, and a non-singleton
    /// top level — the exact applicability conditions of the theorem).
    pub fn validate(&self) -> Result<()> {
        let n = self.coordinates;
        if self.cols.checked_mul(self.rows) != Some(n) {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "checkpoint plan: cols*rows ({}) != coordinates ({n})",
                    self.cols * self.rows
                ),
            ));
        }
        if self.partitions.is_empty() {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                "checkpoint plan has no partition levels",
            ));
        }
        if let Some(bad) = self.partitions.iter().find(|p| p.len() != n) {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "checkpoint partition length {} != coordinates {n}",
                    bad.len()
                ),
            ));
        }
        api::create_hierarchy(n, &self.partitions)?;
        api::create_group(self.group_modulus)?;
        Ok(())
    }

    /// Number of checkpoints (`q + 1`).
    pub fn levels(&self) -> usize {
        self.partitions.len()
    }

    /// The proved worst-case bound `q + 1`.
    pub fn worst_case_bound(&self) -> usize {
        self.levels()
    }

    /// Encode a coordinate vector (row-major tile sizes) into its persistent
    /// word and decode every checkpoint from its own prefix.
    pub fn encode(&self, coordinates: &[u64]) -> Result<CheckpointIndex> {
        let (enc, word) = self.encode_word(coordinates)?;

        let report = api::verify_word(&enc, &word);
        if !report.ok {
            return Err(CoreError::internal(
                InternalCode::Invariant,
                format!("math encoder produced an invalid word: {}", report.message),
            ));
        }
        let decoded = api::decode_full(&enc, &word)?;
        if decoded != coordinates {
            return Err(CoreError::internal(
                InternalCode::Invariant,
                "math encoder did not round-trip the coordinate vector",
            ));
        }

        let mut views = Vec::with_capacity(enc.hierarchy().num_levels());
        for t in 0..enc.hierarchy().num_levels() {
            // `decode_checkpoint` is prefix-only by construction (it reads
            // `word[0..b_t]`); the prototype test suite pins that property.
            views.push(api::decode_checkpoint(&enc, &word, t)?.block_sums);
        }
        let gate = api::applicability_gate(&enc);

        Ok(CheckpointIndex {
            group_modulus: self.group_modulus,
            coordinates: self.coordinates,
            hierarchy_depth: enc.hierarchy().q(),
            expected_upper_bound: enc.worst_case_bound(),
            checkpoint_bounds: enc.layout().checkpoint_bounds().to_vec(),
            checkpoint_views: views,
            word,
            source: coordinates.to_vec(),
            prefix_semantics: enc.prefix_semantics().to_string(),
            proof_assumption_version: enc.proof_assumption_version().to_string(),
            proof_bound_applicable: enc.proof_bound_applicable(),
            gate_all_pass: gate.all_pass(),
        })
    }

    fn encoder(&self) -> Result<carryon_math_core::encoder::Encoder> {
        let group = api::create_group(self.group_modulus)?;
        let hierarchy = api::create_hierarchy(self.coordinates, &self.partitions)?;
        Ok(api::create_all_but_one_encoder(
            group,
            hierarchy,
            OmissionPolicy::LastChild,
        ))
    }

    fn encode_word(
        &self,
        coordinates: &[u64],
    ) -> Result<(carryon_math_core::encoder::Encoder, Vec<u64>)> {
        if coordinates.len() != self.coordinates {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "checkpoint coordinate count {} != plan coordinates {}",
                    coordinates.len(),
                    self.coordinates
                ),
            ));
        }
        let enc = self.encoder()?;
        let word = api::encode(&enc, coordinates)?;
        Ok((enc, word))
    }

    /// Prefix `[0, b_t)` of a word — the bytes a receiver needs for checkpoint `t`.
    pub fn prefix<'a>(&self, word: &'a [u64], level: usize) -> Result<&'a [u64]> {
        if level >= self.levels() {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "checkpoint level {level} out of range [0, {})",
                    self.levels()
                ),
            ));
        }
        let enc = self.encoder()?;
        let b = enc.layout().checkpoint_bound(level);
        if word.len() != self.coordinates {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "word length {} != coordinates {}",
                    word.len(),
                    self.coordinates
                ),
            ));
        }
        Ok(&word[..b])
    }

    /// Independent reference block sums `F_0 … F_q` computed directly from the
    /// source vector and the partitions (no encoder). A checkpoint decoded from
    /// a prefix must equal this exactly.
    pub fn reference_views(&self, coordinates: &[u64]) -> Result<Vec<Vec<u64>>> {
        if coordinates.len() != self.coordinates {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "checkpoint coordinate count {} != plan coordinates {}",
                    coordinates.len(),
                    self.coordinates
                ),
            ));
        }
        if coordinates.iter().any(|&v| v >= self.group_modulus) {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                "coordinate outside the group alphabet (>= modulus)",
            ));
        }
        let mut views = Vec::with_capacity(self.partitions.len());
        for part in &self.partitions {
            // Canonical block ids in order of each block's smallest coordinate —
            // the same ordering `carryon_math_core` assigns.
            let mut tag_to_id: BTreeMap<usize, usize> = BTreeMap::new();
            for &tag in part.iter().take(self.coordinates) {
                let next = tag_to_id.len();
                tag_to_id.entry(tag).or_insert(next);
            }
            let mut sums = vec![0u64; tag_to_id.len()];
            for (i, &tag) in part.iter().enumerate() {
                let b = tag_to_id[&tag];
                sums[b] = (sums[b] + coordinates[i]) % self.group_modulus;
            }
            views.push(sums);
        }
        Ok(views)
    }

    /// Measure one single-coordinate update `x_i ← x_i + delta` (the theorem's
    /// unit). Fails closed if the proven bound is violated.
    pub fn single_update(
        &self,
        index: &CheckpointIndex,
        coordinate: usize,
        delta: u64,
    ) -> Result<SingleUpdate> {
        if index.coordinates != self.coordinates {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                "checkpoint index does not belong to this plan",
            ));
        }
        let enc = self.encoder()?;
        let update = api::apply_coordinate_delta(&enc, &index.word, coordinate, delta)?;
        let changed = update.changed_symbol_indices.clone();

        let mut prefix_changed = Vec::with_capacity(self.levels());
        let mut prefix_ok = true;
        for t in 0..self.levels() {
            let b = enc.layout().checkpoint_bound(t);
            let c = changed.iter().take_while(|&&s| s < b).count();
            // main.tex eq. (13): the level-t hard edge changes at least t+1
            // symbols in the nested prefixes for *some* edge; the canonical
            // construction changes at most one stored symbol per refinement
            // level, so any single update changes at most t+1 inside b_t.
            if c > t + 1 {
                prefix_ok = false;
            }
            prefix_changed.push(c);
        }
        let bound_holds =
            update.endpoint_changed_symbols <= update.expected_upper_bound && prefix_ok;
        if !bound_holds {
            return Err(CoreError::internal(
                InternalCode::Invariant,
                format!(
                    "math checkpoint bound violated: changed {} > q+1 = {}",
                    update.endpoint_changed_symbols, update.expected_upper_bound
                ),
            ));
        }

        Ok(SingleUpdate {
            index: coordinate,
            delta,
            changed_symbols: update.endpoint_changed_symbols,
            changed_symbol_indices: changed,
            upper_bound: update.expected_upper_bound,
            bound_holds,
            prefix_changed,
            prefix_change_bound_holds: prefix_ok,
        })
    }

    /// Measure endpoint dilation over **every** single-coordinate update with
    /// `delta = 1` (a one-byte size change is the smallest legal update).
    pub fn dilation(&self, index: &CheckpointIndex) -> Result<DilationReport> {
        let mut histogram: BTreeMap<usize, usize> = BTreeMap::new();
        let mut max = 0usize;
        let mut min = usize::MAX;
        let mut total = 0usize;
        let mut attaining = Vec::new();
        for i in 0..self.coordinates {
            let u = self.single_update(index, i, 1)?;
            histogram
                .entry(u.changed_symbols)
                .and_modify(|c| *c += 1)
                .or_insert(1);
            if u.changed_symbols > max {
                max = u.changed_symbols;
                attaining.clear();
                attaining.push(i);
            } else if u.changed_symbols == max {
                attaining.push(i);
            }
            min = min.min(u.changed_symbols);
            total += u.changed_symbols;
        }
        let bounds = &index.checkpoint_bounds;
        let prefix_bytes_u64 = bounds.iter().map(|b| b * 8).collect();
        Ok(DilationReport {
            coordinates: self.coordinates,
            hierarchy_depth: index.hierarchy_depth,
            upper_bound: index.expected_upper_bound,
            measured_max: max,
            measured_min: min,
            measured_mean_milli: total
                .saturating_mul(1000)
                .checked_div(self.coordinates)
                .unwrap_or(0),
            attaining_indices: attaining,
            histogram,
            bound_holds: max <= index.expected_upper_bound,
            prefix_bytes_u64,
        })
    }

    /// Endpoint Hamming distance between two words. For a **single-coordinate**
    /// update from a word produced by this plan, `<= q + 1` (the theorem); a
    /// multi-coordinate update has no such bound and is reported as-is.
    pub fn endpoint_distance(a: &[u64], b: &[u64]) -> usize {
        a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
    }
}

/// Build the coarse→fine `block_of` assignments of the tile pyramid.
///
/// Candidate refinements are tried in a fixed order and kept only when every
/// current block splits into at least two candidate blocks — the strictness
/// condition of (1). The cell assignment is always reached, so the finest level
/// is the singleton partition; for a 1×1 grid the result is the single
/// degenerate level (`q = 0`).
fn pyramid_partitions(cols: usize, rows: usize) -> Vec<Vec<usize>> {
    let n = cols * rows;
    let mut levels: Vec<Vec<usize>> = vec![vec![0usize; n]];
    let mut current = vec![0usize; n];

    let try_level = |cand: Vec<usize>, levels: &mut Vec<Vec<usize>>, current: &mut Vec<usize>| {
        // A non-final level may not contain singleton blocks: such a block could
        // never be split again, so the hierarchy could not reach singletons.
        let is_final = is_singleton_partition(&cand);
        if !is_final && !all_blocks_have_at_least_two(&cand) {
            return;
        }
        if strictly_refines(current, &cand) {
            *current = cand.clone();
            levels.push(cand);
        }
    };

    // Row bands: two halves of the row range.
    if rows >= 2 {
        let half = rows / 2;
        let bands: Vec<usize> = (0..n)
            .map(|i| {
                let r = i / cols;
                if r < half {
                    0
                } else {
                    1
                }
            })
            .collect();
        try_level(bands, &mut levels, &mut current);
    }
    // Individual rows.
    if rows >= 2 {
        let by_row: Vec<usize> = (0..n).map(|i| i / cols).collect();
        try_level(by_row, &mut levels, &mut current);
    }
    // Two column halves within each current block.
    if cols >= 2 {
        let half = cols / 2;
        let halves: Vec<usize> = (0..n)
            .map(|i| {
                let (r, c) = (i / cols, i % cols);
                r * 2 + usize::from(c >= half)
            })
            .collect();
        try_level(halves, &mut levels, &mut current);
    }
    // Cells (singletons) — the finest level.
    let cells: Vec<usize> = (0..n).collect();
    try_level(cells, &mut levels, &mut current);
    levels
}

/// Whether `cand` is a strict refinement of `current`: every candidate block
/// lies inside one current block, and every current block contains at least two
/// distinct candidate blocks.
fn strictly_refines(current: &[usize], cand: &[usize]) -> bool {
    if current.len() != cand.len() || current == cand {
        return false;
    }
    let mut children_of: BTreeMap<usize, std::collections::BTreeSet<usize>> = BTreeMap::new();
    let mut parents_of: BTreeMap<usize, std::collections::BTreeSet<usize>> = BTreeMap::new();
    for (&p, &c) in current.iter().zip(cand.iter()) {
        children_of.entry(p).or_default().insert(c);
        parents_of.entry(c).or_default().insert(p);
    }
    // Refinement: no candidate block spans two current blocks.
    if parents_of.values().any(|ps| ps.len() > 1) {
        return false;
    }
    // Strictness: every current block splits into >= 2 candidate blocks.
    children_of.values().all(|cs| cs.len() >= 2)
}

/// Every distinct block tag occurs exactly once (the singleton partition).
fn is_singleton_partition(block_of: &[usize]) -> bool {
    let distinct: std::collections::BTreeSet<usize> = block_of.iter().copied().collect();
    distinct.len() == block_of.len()
}

/// No block is a singleton — i.e. the level can still be strictly refined.
fn all_blocks_have_at_least_two(block_of: &[usize]) -> bool {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for &b in block_of {
        *counts.entry(b).or_insert(0) += 1;
    }
    counts.values().all(|&c| c >= 2)
}

// --- engine hook: derive the coordinate vector from a sealed cut -------------

impl Core {
    /// The authoritative object sizes of a sealed cut, in cut ordinal order.
    ///
    /// `object_id_prefix` optionally restricts the vector to a namespace (e.g.
    /// `photocraft.tile.` for the real tile pyramid); `None` means every
    /// authoritative object. This is the cut's source state for the block-sum
    /// index — real engine rows, never adapter-supplied bytes.
    pub fn cut_checkpoint_coordinates(
        &self,
        cut: CutId,
        object_id_prefix: Option<&str>,
    ) -> Result<Vec<u64>> {
        let conn = self.db_shared().conn();
        let mut stmt = conn
            .prepare(
                "SELECT o.object_id, o.logical_size \
                 FROM cut_objects c \
                 JOIN objects o ON o.object_id = c.object_id AND o.generation = c.generation \
                 WHERE c.session_id = ?1 AND c.cut_number = ?2 AND c.role = 'authoritative' \
                 ORDER BY c.ordinal",
            )
            .map_err(|e| {
                CoreError::schema(SchemaCode::Invalid, format!("checkpoint query: {e}"))
            })?;
        let rows = stmt
            .query_map(
                rusqlite::params![cut.session.to_string(), cut.number as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(|e| {
                CoreError::schema(SchemaCode::Invalid, format!("checkpoint query: {e}"))
            })?;
        let mut sizes = Vec::new();
        for row in rows {
            let (object_id, size) =
                row.map_err(|e| CoreError::schema(SchemaCode::Invalid, format!("{e}")))?;
            if object_id_prefix.is_some_and(|p| !object_id.starts_with(p)) {
                continue;
            }
            if size < 0 {
                return Err(CoreError::internal(
                    InternalCode::Invariant,
                    format!("object {object_id} has a negative logical_size {size}"),
                ));
            }
            sizes.push(size as u64);
        }
        Ok(sizes)
    }

    /// Encode the block-sum checkpoint index of a sealed cut under `plan`.
    ///
    /// The cut's authoritative sizes (optionally filtered by object-id prefix)
    /// are the coordinates; their count must match `plan.coordinates`. The word
    /// is a derived view — nothing is persisted and no production path calls
    /// this unless the `blocksum-checkpoint` feature is enabled.
    pub fn checkpoint_index(
        &self,
        cut: CutId,
        plan: &CheckpointPlan,
        object_id_prefix: Option<&str>,
    ) -> Result<CheckpointIndex> {
        let coordinates = self.cut_checkpoint_coordinates(cut, object_id_prefix)?;
        if coordinates.len() != plan.coordinates {
            return Err(CoreError::schema(
                SchemaCode::Invalid,
                format!(
                    "cut {}:{} has {} authoritative coordinates, plan expects {}",
                    cut.session,
                    cut.number,
                    coordinates.len(),
                    plan.coordinates
                ),
            ));
        }
        plan.encode(&coordinates)
    }

    /// Measure endpoint dilation over every single-coordinate update of a sealed
    /// cut's checkpoint index. The theorem's unit: one object changes by one byte.
    pub fn checkpoint_dilation(
        &self,
        cut: CutId,
        plan: &CheckpointPlan,
        object_id_prefix: Option<&str>,
    ) -> Result<DilationReport> {
        let index = self.checkpoint_index(cut, plan, object_id_prefix)?;
        plan.dilation(&index)
    }

    /// Endpoint distance between the checkpoint words of two sealed cuts of the
    /// same session. A multi-coordinate change: reported, not bounded by `q+1`.
    pub fn checkpoint_endpoint_distance(
        &self,
        from: CutId,
        to: CutId,
        plan: &CheckpointPlan,
        object_id_prefix: Option<&str>,
    ) -> Result<usize> {
        let a = self.checkpoint_index(from, plan, object_id_prefix)?;
        let b = self.checkpoint_index(to, plan, object_id_prefix)?;
        Ok(CheckpointPlan::endpoint_distance(&a.word, &b.word))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_are_strict_for_the_real_grid() {
        let plan = CheckpointPlan::tile_pyramid(10, 6).unwrap();
        // whole -> 2 row bands -> 6 rows -> 12 half-rows -> 60 cells
        assert_eq!(plan.partitions.len(), 5);
        assert_eq!(plan.worst_case_bound(), 5);
        assert_eq!(plan.partitions.last().unwrap().len(), 60);
        assert!(api::create_hierarchy(60, &plan.partitions).is_ok());
    }

    #[test]
    fn degenerate_grids_are_valid() {
        for (c, r) in [(1, 1), (1, 6), (10, 1), (2, 2), (3, 3), (8, 8)] {
            let plan = CheckpointPlan::tile_pyramid(c, r).unwrap();
            assert_eq!(plan.coordinates, c * r);
            assert!(plan.worst_case_bound() >= 1);
        }
    }

    #[test]
    fn encode_round_trips_and_gates() {
        let plan = CheckpointPlan::tile_pyramid(10, 6).unwrap();
        let coords: Vec<u64> = (0..60).map(|i| (i as u64) * 7 % 1000).collect();
        let idx = plan.encode(&coords).unwrap();
        assert_eq!(idx.source, coords);
        assert_eq!(idx.prefix_semantics, "view-pure");
        assert!(idx.proof_bound_applicable);
        assert!(idx.gate_all_pass);
        assert_eq!(idx.checkpoint_bounds, vec![1, 2, 6, 12, 60]);
        // F_0 is the total byte sum.
        assert_eq!(idx.checkpoint_views[0], vec![coords.iter().sum::<u64>()]);
    }

    #[test]
    fn single_update_never_exceeds_q_plus_one() {
        let plan = CheckpointPlan::tile_pyramid(10, 6).unwrap();
        let coords: Vec<u64> = (0..60).map(|i| 1000 + (i as u64) * 13).collect();
        let idx = plan.encode(&coords).unwrap();
        for i in 0..60 {
            let u = plan.single_update(&idx, i, 1).unwrap();
            assert!(u.bound_holds);
            assert!(u.changed_symbols <= 5);
            assert!(u.prefix_change_bound_holds);
        }
    }

    #[test]
    fn some_coordinate_attains_the_bound() {
        let plan = CheckpointPlan::tile_pyramid(10, 6).unwrap();
        let coords: Vec<u64> = (0..60).map(|i| 1000 + i as u64).collect();
        let idx = plan.encode(&coords).unwrap();
        let report = plan.dilation(&idx).unwrap();
        assert!(report.bound_holds);
        assert_eq!(
            report.measured_max, report.upper_bound,
            "the construction attains q+1"
        );
        assert!(!report.attaining_indices.is_empty());
    }

    #[test]
    fn view_purity_prefixes_depend_only_on_block_sums() {
        // For every checkpoint level, two source vectors with the SAME block
        // sums at that level must share the length-b_t prefix (view-purity).
        let plan = CheckpointPlan::tile_pyramid(4, 3).unwrap();
        let base: Vec<u64> = vec![100; 12];
        for t in 0..plan.levels() {
            // Find two coordinates in one block of level t.
            let mut pair = None;
            for block in 0..plan.partitions[t].iter().max().copied().unwrap_or(0) + 1 {
                let coords: Vec<usize> = (0..12)
                    .filter(|&i| plan.partitions[t][i] == block)
                    .collect();
                if coords.len() >= 2 {
                    pair = Some((coords[0], coords[1]));
                    break;
                }
            }
            let (u, v) = match pair {
                Some(p) => p,
                // The finest level is the singleton partition: no block has two
                // coordinates, so there is no equal-view pair to test.
                None => continue,
            };
            let mut a = base.clone();
            let mut b = base.clone();
            a[u] += 1;
            b[v] += 1;
            let ia = plan.encode(&a).unwrap();
            let ib = plan.encode(&b).unwrap();
            // F_t(a) == F_t(b) by construction (same block, +1 applied once).
            assert_eq!(
                ia.checkpoint_views[t], ib.checkpoint_views[t],
                "level {t} views must agree"
            );
            assert_eq!(
                plan.prefix(&ia.word, t).unwrap(),
                plan.prefix(&ib.word, t).unwrap(),
                "level {t} prefix must be identical"
            );
            assert_ne!(ia.word, ib.word, "the full state (and word) differs");
        }
    }

    #[test]
    fn multi_coordinate_distance_is_reported_not_bounded() {
        let plan = CheckpointPlan::tile_pyramid(10, 6).unwrap();
        let a: Vec<u64> = vec![100; 60];
        let mut b = a.clone();
        b[0] += 1;
        b[59] += 1;
        let ia = plan.encode(&a).unwrap();
        let ib = plan.encode(&b).unwrap();
        let d = CheckpointPlan::endpoint_distance(&ia.word, &ib.word);
        assert!(d >= 1, "distinct states must produce distinct words");
        assert!(
            d <= 2 * plan.worst_case_bound(),
            "two single-update bounds cannot compose beyond 2(q+1)"
        );
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Any grid (including degenerate ones), any real size vector, any
        /// coordinate, any nonzero delta: the endpoint update stays inside the
        /// proven `q + 1` bound and every prefix stays inside its `t + 1` bound.
        #[test]
        fn any_grid_any_update_respects_the_proven_bounds(
            grid in (1usize..=8, 1usize..=8),
            draw in any::<usize>(),
            draw_delta in 1u64..1_000,
        ) {
            let (cols, rows) = grid;
            let n = cols * rows;
            // A small deterministic real-ish vector: tile sizes in bytes.
            let coords: Vec<u64> = (0..n).map(|i| 4_096 + ((i as u64) * 997) % 200_000).collect();
            let plan = CheckpointPlan::tile_pyramid(cols, rows).unwrap();
            let idx = plan.encode(&coords).unwrap();
            let i = draw % n;
            let u = plan.single_update(&idx, i, draw_delta).unwrap();
            prop_assert!(u.bound_holds);
            prop_assert!(u.changed_symbols <= plan.worst_case_bound());
            prop_assert!(u.prefix_change_bound_holds);
        }

        /// View-purity: for any grid, any two vectors whose level-`t` block sums
        /// agree must share the length-`b_t` prefix — for every checkpoint.
        #[test]
        fn equal_block_sums_imply_equal_prefixes(
            grid in (1usize..=6, 1usize..=6),
            bump in 1u64..500,
        ) {
            let (cols, rows) = grid;
            let n = cols * rows;
            let plan = CheckpointPlan::tile_pyramid(cols, rows).unwrap();
            let base: Vec<u64> = vec![10_000; n];
            for t in 0..plan.levels() {
                // Find one level-t block with at least two coordinates.
                let blocks = plan.partitions[t].iter().max().copied().unwrap_or(0) + 1;
                let pair = (0..blocks).find_map(|b| {
                    let cs: Vec<usize> = (0..n).filter(|&i| plan.partitions[t][i] == b).collect();
                    (cs.len() >= 2).then(|| (cs[0], cs[1]))
                });
                let Some((u, v)) = pair else { continue };
                let mut a = base.clone();
                let mut b = base.clone();
                a[u] += bump;
                b[v] += bump;
                let ia = plan.encode(&a).unwrap();
                let ib = plan.encode(&b).unwrap();
                prop_assert_eq!(ia.checkpoint_views[t].clone(), ib.checkpoint_views[t].clone());
                prop_assert_eq!(
                    plan.prefix(&ia.word, t).unwrap(),
                    plan.prefix(&ib.word, t).unwrap()
                );
            }
        }
    }
}
