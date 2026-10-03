//! Bridge the PROVED block-sum encoder (`carryon-math-core`) into the engine as a
//! progressive/checkpoint object view (spec §4.3 progressive inspection, §5.10
//! manifest labels, §23.2 math metrics).
//!
//! An adapter that holds per-coordinate integer counters (e.g. per-node visit
//! counts) models them as `x ∈ (Z/kZ)^n` under a strict hierarchy, and the
//! engine can expose each checkpoint view `F_0..F_q` as a Preview object and
//! record the exact §23.2 math metrics. No changes are made to `carryon-math-core`;
//! it is used only through its `api` facade.

use crate::error::{CoreError, Result};
use carryon_math_core::api;
use carryon_math_core::layout::OmissionPolicy;
use serde::{Deserialize, Serialize};

/// The block-sum view of a counter vector, ready to become Preview objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSumView {
    /// The encoder manifest labels (§5.10).
    pub prefix_semantics: String,
    pub proof_assumption_version: String,
    pub proof_bound_applicable: bool,
    /// Whether the §5.11 applicability gate passed (MATH-007).
    pub gate_all_pass: bool,
    pub hierarchy_depth: usize,
    /// Checkpoint prefix lengths `b_0..b_q`.
    pub checkpoint_bounds: Vec<usize>,
    /// The per-level block sums `F_0..F_q`.
    pub checkpoint_views: Vec<Vec<u64>>,
}

/// Metrics recorded for a single-counter update (§23.2 math encoder block).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MathMetrics {
    pub endpoint_changed_symbols: usize,
    pub changed_symbol_indices: Vec<usize>,
    pub expected_upper_bound: usize,
    pub proof_bound_applicable: bool,
    pub hierarchy_depth: usize,
    pub checkpoint_bounds: Vec<usize>,
}

/// Encode a counter vector under a hierarchy and return its block-sum view.
///
/// `partitions` is the ordered coarse→fine partition assignment (same form as
/// `carryon_math_core::api::create_hierarchy`), `k` the group modulus.
pub fn encode_view(k: u64, counters: &[u64], partitions: &[Vec<usize>]) -> Result<BlockSumView> {
    let group = api::create_group(k)?;
    let hierarchy = api::create_hierarchy(counters.len(), partitions)?;
    let enc = api::create_all_but_one_encoder(group, hierarchy, OmissionPolicy::LastChild);
    let word = api::encode(&enc, counters)?;

    let gate = api::applicability_gate(&enc);
    let mut views = Vec::new();
    for t in 0..=enc.hierarchy().q() {
        views.push(api::decode_checkpoint(&enc, &word, t)?.block_sums);
    }

    Ok(BlockSumView {
        prefix_semantics: enc.prefix_semantics().to_string(),
        proof_assumption_version: enc.proof_assumption_version().to_string(),
        proof_bound_applicable: enc.proof_bound_applicable(),
        gate_all_pass: gate.all_pass(),
        hierarchy_depth: enc.hierarchy().q(),
        checkpoint_bounds: enc.layout().checkpoint_bounds().to_vec(),
        checkpoint_views: views,
    })
}

/// Apply a single-counter delta and return the §23.2 math metrics. The returned
/// `endpoint_changed_symbols ≤ expected_upper_bound = q + 1` always holds for the
/// view-pure construction (spec §5.8/§5.9).
pub fn update_metrics(
    k: u64,
    counters: &[u64],
    partitions: &[Vec<usize>],
    index: usize,
    delta: u64,
) -> Result<MathMetrics> {
    let group = api::create_group(k)?;
    let hierarchy = api::create_hierarchy(counters.len(), partitions)?;
    let enc = api::create_all_but_one_encoder(group, hierarchy, OmissionPolicy::LastChild);
    let word = api::encode(&enc, counters)?;
    let upd = api::apply_coordinate_delta(&enc, &word, index, delta)?;

    // Sanity: the proven bound must hold. A violation is a core invariant failure.
    if upd.endpoint_changed_symbols > upd.expected_upper_bound {
        return Err(CoreError::internal(
            crate::error::InternalCode::Invariant,
            "math encoder exceeded proven q+1 bound",
        ));
    }

    Ok(MathMetrics {
        endpoint_changed_symbols: upd.endpoint_changed_symbols,
        changed_symbol_indices: upd.changed_symbol_indices,
        expected_upper_bound: upd.expected_upper_bound,
        proof_bound_applicable: upd.proof_bound_applicable,
        hierarchy_depth: enc.hierarchy().q(),
        checkpoint_bounds: enc.layout().checkpoint_bounds().to_vec(),
    })
}
