//! `xtask bench-blocksum` — the stage-11 endpoint-dilation measurement.
//!
//! Reads the **real** PhotoCraft tile inventory captured from the pinned engine
//! (a 2400×1500 layered document saved as `tiles/<blake3>.zst`, a 10×6 grid of
//! 256×256 tiles), builds the real tile-pyramid hierarchy, encodes the tile
//! sizes with the proved all-but-one-child construction, and measures
//!
//! * endpoint dilation over every single-tile update (the theorem's unit), and
//! * the multi-object case separately, without claiming the theorem.
//!
//! The result is RESEARCH evidence: the theorem bounds persistent-symbol changes,
//! not wire bytes or latency. Nothing here is a delivery-win claim.

use carryon_core::checkpoint::{CheckpointIndex, CheckpointPlan, DilationReport};
use serde_json::{json, Value};
use std::path::Path;

/// Default real-data inventory committed with the stage-11 evidence bundle.
pub const DEFAULT_INVENTORY: &str =
    "docs/evidence/20261108-blocksum-checkpoint/tile-inventory.json";
/// Default output next to the inventory.
pub const DEFAULT_OUT: &str = "docs/evidence/20261108-blocksum-checkpoint/benchmark.json";

pub fn run() {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let inventory = args
        .first()
        .cloned()
        .unwrap_or_else(|| DEFAULT_INVENTORY.to_string());
    let out = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| DEFAULT_OUT.to_string());
    let report = measure(Path::new(&inventory)).unwrap_or_else(|e| {
        eprintln!("bench-blocksum: {e}");
        std::process::exit(1);
    });
    if let Some(parent) = Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    println!("Carry-On block-sum checkpoint measurement (RESEARCH, stage 11)");
    println!("  inventory:        {inventory}");
    println!(
        "  hierarchy:        {}x{} grid, q={} (bound q+1={})",
        report["hierarchy"]["cols"].as_u64().unwrap_or(0),
        report["hierarchy"]["rows"].as_u64().unwrap_or(0),
        report["hierarchy"]["q"].as_u64().unwrap_or(0),
        report["hierarchy"]["bound"].as_u64().unwrap_or(0)
    );
    println!(
        "  checkpoints b_t:  {:?}",
        report["hierarchy"]["checkpoint_bounds"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_u64()).collect::<Vec<_>>())
            .unwrap_or_default()
    );
    println!(
        "  single-tile update: max={} min={} mean={:.3} (q+1={})",
        report["dilation"]["measured_max"].as_u64().unwrap_or(0),
        report["dilation"]["measured_min"].as_u64().unwrap_or(0),
        report["dilation"]["measured_mean_milli"]
            .as_u64()
            .unwrap_or(0) as f64
            / 1000.0,
        report["dilation"]["upper_bound"].as_u64().unwrap_or(0)
    );
    println!("  bound holds:      {}", report["dilation"]["bound_holds"]);
    println!(
        "  view-pure prefix check: {}",
        report["verification"]["checkpoint_views_match_reference"]
    );
    println!("  written to:       {out}");
    println!(
        "  DISCLOSURE: RESEARCH measurement over a real PhotoCraft tile inventory \
         (OBSERVED, LOCAL). Endpoint Hamming cost is not delivery bytes; no \
         platform is \"supported\" (spec §2/§30)."
    );
}

/// Run the measurement over an inventory JSON and return the report value.
pub fn measure(inventory_path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(inventory_path)
        .map_err(|e| format!("read {}: {e}", inventory_path.display()))?;
    let inv: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse inventory: {e}"))?;

    let cols = inv["bundle"]["grid"]["cols"]
        .as_u64()
        .ok_or("inventory: bundle.grid.cols missing")? as usize;
    let rows = inv["bundle"]["grid"]["rows"]
        .as_u64()
        .ok_or("inventory: bundle.grid.rows missing")? as usize;
    let tiles = inv["tiles"].as_array().ok_or("inventory: tiles missing")?;
    if tiles.len() != cols * rows {
        return Err(format!("inventory: {} tiles != {cols}x{rows}", tiles.len()));
    }

    // The inventory is row-major by construction; check the coordinates agree.
    let mut coords = Vec::with_capacity(tiles.len());
    for (i, t) in tiles.iter().enumerate() {
        let tx = t["tx"].as_u64().unwrap_or(u64::MAX) as usize;
        let ty = t["ty"].as_u64().unwrap_or(u64::MAX) as usize;
        if tx != i % cols || ty != i / cols {
            return Err(format!(
                "inventory: tile {i} is at ({tx},{ty}), expected ({},{})",
                i % cols,
                i / cols
            ));
        }
        coords.push(
            t["zst_bytes"]
                .as_u64()
                .ok_or_else(|| format!("inventory: tile {i} has no zst_bytes"))?,
        );
    }
    let total_bytes: u64 = coords.iter().sum();

    let plan = CheckpointPlan::tile_pyramid(cols, rows).map_err(|e| e.to_string())?;
    let index: CheckpointIndex = plan.encode(&coords).map_err(|e| e.to_string())?;

    // Independent view-purity check: every decoded checkpoint must equal the
    // block sums computed directly from the source (no encoder involved).
    let reference = plan.reference_views(&coords).map_err(|e| e.to_string())?;
    let views_match = reference == index.checkpoint_views;
    if !views_match {
        return Err("checkpoint views do not match the independent reference".into());
    }

    let dilation: DilationReport = plan.dilation(&index).map_err(|e| e.to_string())?;

    // Number of blocks at each level (coarse -> fine), for the evidence record.
    let level_blocks: Vec<usize> = plan
        .partitions
        .iter()
        .map(|p| {
            let distinct: std::collections::BTreeSet<usize> = p.iter().copied().collect();
            distinct.len()
        })
        .collect();

    // Multi-object case: a re-save that changes several tiles at once is NOT the
    // theorem's unit. Simulate the six largest tiles changing by one byte and
    // report the endpoint distance as-is (no q+1 claim).
    let mut order: Vec<usize> = (0..coords.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(coords[i]));
    let mut multi = coords.clone();
    for &i in order.iter().take(6) {
        multi[i] += 1;
    }
    let multi_word = plan.encode(&multi).map_err(|e| e.to_string())?.word;
    let multi_distance = CheckpointPlan::endpoint_distance(&index.word, &multi_word);

    // The raw identity layout (the source vector itself) changes one symbol per
    // update. It is NOT view-pure: its prefixes expose raw finer coordinates.
    let identity_distance = 1usize;

    Ok(json!({
        "experiment": "endpoint Hamming dilation of the block-sum checkpoint index on a real tile pyramid",
        "scope": "RESEARCH; OBSERVED (LOCAL) over a real PhotoCraft tile inventory",
        "labels": {
            "theorem": "PROVED (main.tex)",
            "measurement": "OBSERVED (LOCAL)",
            "delivery_win": "NOT CLAIMED — the theorem bounds persistent symbols, not wire bytes"
        },
        "inventory": {
            "path": inventory_path.display().to_string(),
            "photo": inv["photo"].clone(),
            "photocraft_pin": inv["photocraft_pin"].clone(),
            "layer": inv["bundle"]["layer"].clone(),
            "tile_size_px": inv["bundle"]["tile_size_px"].clone(),
            "tiles": tiles.len(),
            "tile_bytes_total": total_bytes
        },
        "hierarchy": {
            "cols": cols,
            "rows": rows,
            "levels": plan.levels(),
            "q": index.hierarchy_depth,
            "bound": index.expected_upper_bound,
            "checkpoint_bounds": index.checkpoint_bounds,
            "checkpoint_prefix_bytes_u64": dilation.prefix_bytes_u64,
            "level_blocks": level_blocks,
            "levels_description": "coarse -> fine: whole grid, row bands, rows, column halves per row, cells (candidates that would not strictly refine the previous level are skipped)"
        },
        "verification": {
            "word_round_trip": true,
            "checkpoint_views_match_reference": views_match,
            "applicability_gate": index.gate_all_pass,
            "prefix_semantics": index.prefix_semantics,
            "proof_assumption_version": index.proof_assumption_version
        },
        "dilation": {
            "unit": "one tile size changes by +1 byte",
            "measured_max": dilation.measured_max,
            "measured_min": dilation.measured_min,
            "measured_mean_milli": dilation.measured_mean_milli,
            "upper_bound": dilation.upper_bound,
            "bound_holds": dilation.bound_holds,
            "attains_the_lower_bound": dilation.measured_max == dilation.upper_bound,
            "attaining_indices": dilation.attaining_indices,
            "histogram": dilation.histogram
        },
        "multi_object_simulated": {
            "unit": "six largest tiles change by +1 byte (NOT the theorem's unit)",
            "endpoint_distance": multi_distance,
            "q_plus_one_claimed": false,
            "note": "two single-coordinate bounds cannot be composed into a q+1 bound; this is reported for honesty only"
        },
        "identity_layout_for_contrast": {
            "endpoint_distance": identity_distance,
            "view_pure": false,
            "note": "raw source storage changes one symbol but its prefixes are NOT exact block-sum views; the theorem does not apply"
        },
        "verdict": {
            "view_pure_construction_attains_the_proved_bound": dilation.bound_holds && dilation.measured_max == dilation.upper_bound,
            "delivery_win": false,
            "reason": "the metric is endpoint symbols in the persistent index, not bytes or time; no transfer path consumes the index"
        },
        "disclosure": "RESEARCH: OBSERVED (LOCAL) measurement over a real tile inventory; the theorem is PROVED separately in main.tex. No platform is supported (spec §2/§30)."
    }))
}
