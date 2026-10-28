//! Real-edit-stream measurement (Phase 3, task 4): journal growth over a long
//! editing session, and where the journal/snapshot crossover sits.
//!
//! HONEST SCOPING: this is a *scripted-but-varied* command mix (brush work
//! with pressure curves, layer ops, adjustments, filters, selections,
//! undo/redo, transforms) — NOT a captured human GUI session (that would need
//! input-event recording over a real user's session; labeled as such). What it
//! gives: journal bytes as a function of edit count on a REAL photo document,
//! vs the (roughly edit-count-independent) snapshot size — the crossover
//! where journal-carry stops beating snapshot-carry.
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --features inprocess \
//!   --example edit_stream -- <photo.jpg> <out.json>
//! ```

use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{hex_sha256, UnsavedMode};
use carryon_adapter_photocraft::PhotocraftDriver;
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let photo = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: edit_stream <photo.jpg> <out.json>"),
    );
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "docs/artcraft-bridge/evidence/edit-stream.json".into());
    let work = std::env::temp_dir().join("pcraft-edit-stream");
    std::fs::create_dir_all(&work).unwrap();
    let photo_in_root = work.join(photo.file_name().unwrap());
    std::fs::copy(&photo, &photo_in_root).unwrap();

    let mut d = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    let rel = photo.file_name().unwrap().to_string_lossy().into_owned();
    d.open_under_root(&rel).unwrap();
    d.session_mut().execute("image.duplicate", json!({})).unwrap();
    d.session_mut()
        .execute("layer.newAdjustmentLayer.brightnessContrast", json!({"brightness": 10}))
        .unwrap();
    d.session_mut()
        .execute("layer.new.layer", json!({"name": "Work"}))
        .unwrap();
    d.session_mut()
        .execute("layer.select", json!({"name": "Work"}))
        .unwrap();
    d.save_active_under_root("session.pcraft").unwrap();

    // A varied, long edit script: 240 steps in 6 blocks of 40, measuring
    // journal + snapshot bytes every 40 steps.
    let mut samples: Vec<serde_json::Value> = Vec::new();
    let (w, h) = (2400u64, 1500u64);
    for step in 0..240u64 {
        let frac = (step % 40) as f64 / 40.0;
        let cx = 200.0 + (w as f64 - 400.0) * ((step * 37 % 240) as f64 / 240.0);
        let cy = 150.0 + (h as f64 - 300.0) * ((step * 53 % 150) as f64 / 150.0);
        match step % 6 {
            // brush strokes with 3-point pressure curves (realistic params)
            0 | 1 => {
                d.session_mut().execute(
                    "paint.stroke",
                    json!({"points": [[cx, cy, 1.0], [cx + 40.0, cy + 25.0, 0.7], [cx + 80.0, cy + 50.0, 0.4]], "size": 60 + (step % 5) * 35, "hardness": 0.3, "color": format!("#{:02x}{:02x}{:02x}", 30 + (step % 7) * 12, 60, 90 + (step % 3) * 20)}),
                )
                .unwrap();
            }
            // bigger soft strokes
            2 => {
                d.session_mut().execute(
                    "paint.stroke",
                    json!({"points": [[cx, cy, 1.0], [cx + 120.0, cy - 40.0, 0.85]], "size": 240, "hardness": 0.1, "color": "#224466"}),
                )
                .unwrap();
            }
            // layer property edits
            3 => {
                d.session_mut().execute(
                    "layer.setProps",
                    json!({"name": "Work", "opacity": 0.4 + 0.5 * frac}),
                )
                .unwrap();
            }
            // selections + a filter
            4 => {
                d.session_mut().execute(
                    "select.rect",
                    json!({"x": cx as u64 % 1800, "y": cy as u64 % 1100, "width": 300, "height": 220}),
                )
                .unwrap();
                d.session_mut().execute("filter.blur.gaussianBlur", json!({"radius": 1 + (step % 3)}))
                    .unwrap();
                d.session_mut().execute("select.deselect", json!({})).unwrap();
            }
            // adjustments + undo/redo churn (realistic hesitation)
            _ => {
                d.session_mut().execute(
                    "layer.newAdjustmentLayer.hueSaturation",
                    json!({"hue": (step % 30) as i64 - 15, "saturation": 8}),
                )
                .unwrap();
                d.session_mut().execute("edit.undo", json!({})).unwrap();
            }
        }
        if (step + 1) % 40 == 0 {
            let journal = d.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
            let snapshot = d.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
            let steps = d.journal_steps().len();
            samples.push(json!({
                "edits": step + 1,
                "journal_steps": steps,
                "journal_bytes": journal.len(),
                "snapshot_bytes": snapshot.len(),
                "journal_over_snapshot": journal.len() as f64 / snapshot.len().max(1) as f64,
            }));
            println!(
                "after {:3} edits: journal {} B ({} steps) vs snapshot {} B ({:.2}x)",
                step + 1,
                journal.len(),
                steps,
                snapshot.len(),
                journal.len() as f64 / snapshot.len().max(1) as f64
            );
        }
    }

    // Crossover: journal_bytes > snapshot_bytes. Report the measured point
    // where the ratio crosses 1.0 (or that it never did in this run).
    let mut crossover = serde_json::Value::Null;
    for pair in samples.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        let ra = a["journal_over_snapshot"].as_f64().unwrap();
        let rb = b["journal_over_snapshot"].as_f64().unwrap();
        if ra < 1.0 && rb >= 1.0 {
            crossover = json!({
                "between_edits": [a["edits"], b["edits"]],
                "ratio_before": ra,
                "ratio_after": rb,
            });
        }
    }

    let meta = d.meta().unwrap();
    let composite = d.composite_bytes_of_active().unwrap();
    let evidence = json!({
        "experiment": "edit-stream journal growth (scripted-but-varied mix, NOT a captured human session)",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "scope": "LOCAL (in-process engine session, one machine)",
        "source_photo": { "name": photo.file_name().unwrap().to_string_lossy(), "dimensions": "2400x1500" },
        "edit_mix": "per 6 steps: 2 pressure-curve brush strokes, 1 large soft stroke, 1 layer-props edit, 1 selection+filter pass, 1 adjustment+undo; 240 total",
        "samples": samples,
        "crossover_journal_exceeds_snapshot": crossover,
        "final_state": {
            "revision": meta.revision,
            "dirty": meta.dirty,
            "composite_sha256": hex_sha256(&composite),
        },
    });
    std::fs::write(&out_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {out_path}");
    println!("crossover: {crossover}");
}
