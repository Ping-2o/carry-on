//! The flagship measurement (Phase 2, task 3): `bytes_before_first_action` —
//! journal-carry vs snapshot-carry on REAL layered documents, built and dirtied
//! in a real embedded PhotoCraft engine session (in-process driver, feature
//! `inprocess`).
//!
//! For each document size class, a fresh session:
//!  1. builds a layered document (`file.new` + layers + adjustment layers);
//!  2. saves it as `.pcraft` (the authoritative document object) and marks the
//!     journal checkpoint;
//!  3. applies an edit script (paint strokes, layer props, adjustment, filter,
//!     selection) through the fail-closed authorize gate — all journaled;
//!  4. measures both carry modes of the UNSAVED state:
//!     - snapshot mode: `image.duplicate` + `file.saveAs` = `.pcraft` bytes of
//!       the dirty state (the Phase-2a control-channel technique);
//!     - journal mode: `Session.journal[checkpoint..]` serialized as
//!       `[[id, params], …]`;
//!  5. replays the journal on a SECOND clean session opened on the saved
//!     document and verifies via `document.inspect` re-derivation that the
//!     replayed revision matches and the replayed dirty snapshot hash equals
//!     the source's (the replay oracle).
//!
//! `bytes_before_first_action` per mode = document bytes + unsaved bytes +
//! meta bytes (the prerequisites of `document.edit`/`session.restore`; the
//! ephemeral workspace is optional and excluded — the engine's progressive
//! carry defers it by design).
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --features inprocess \
//!   --example journal_vs_snapshot -- <out.json> [workdir]
//! ```

use carryon_adapter_photocraft::inprocess::{authorize_policy, InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{hex_sha256, PhotocraftMeta, UnsavedMode};
use carryon_adapter_photocraft::{PhotocraftDriver, Step};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Spec {
    name: &'static str,
    w: u32,
    h: u32,
    layers: usize,
    adjustments: usize,
    strokes: usize,
}

fn main() {
    let out_path = std::env::args().nth(1).unwrap_or_else(|| {
        "docs/artcraft-bridge/evidence/journal-vs-snapshot.json".into()
    });
    let work = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let work = work.join("pcraft-measure");
    std::fs::create_dir_all(&work).unwrap();

    let specs = [
        Spec { name: "small-simple", w: 256, h: 256, layers: 1, adjustments: 0, strokes: 3 },
        Spec { name: "medium-layered-adjustments", w: 1024, h: 1024, layers: 6, adjustments: 2, strokes: 15 },
        Spec { name: "large-multi-layer", w: 4096, h: 4096, layers: 12, adjustments: 3, strokes: 30 },
    ];

    let mut results: Vec<Value> = Vec::new();
    for spec in &specs {
        println!("=== {} ({}x{}, {} layers, {} adjustments, {} strokes)",
            spec.name, spec.w, spec.h, spec.layers, spec.adjustments, spec.strokes);

        let mut driver = InProcessDriver::new(InProcessConfig {
            work_root: work.clone(),
        })
        .unwrap();

        // 1. Build the layered document.
        driver
            .session_mut()
            .execute(
                "file.new",
                json!({"width": spec.w, "height": spec.h, "mode": "rgb", "depth": 8, "background": "white"}),
            )
            .unwrap();
        for i in 0..spec.layers {
            driver
                .session_mut()
                .execute("layer.new.layer", json!({"name": format!("Layer {i}")}))
                .unwrap();
            driver
                .session_mut()
                .execute("layer.select", json!({"name": format!("Layer {i}")}))
                .unwrap();
        }
        for a in 0..spec.adjustments {
            driver
                .session_mut()
                .execute(
                    if a % 2 == 0 { "layer.newAdjustmentLayer.brightnessContrast" } else { "layer.newAdjustmentLayer.hueSaturation" },
                    json!({"brightness": 10, "hue": 20, "saturation": 5}),
                )
                .unwrap();
        }

        // 2. Save the document object; mark the journal checkpoint (the
        //    save/dirty boundary).
        let doc_path = work.join(format!("{}.pcraft", spec.name));
        driver
            .session_mut()
            .execute("file.saveAs", json!({"path": doc_path.to_string_lossy()}))
            .unwrap();
        driver.mark_saved_checkpoint();

        // 3. The edit script — journaled, through the authorize gate.
        let cx = spec.w as f64 / 2.0;
        let cy = spec.h as f64 / 2.0;
        for s in 0..spec.strokes {
            let layer = s % spec.layers.max(1);
            driver
                .session_mut()
                .execute("layer.select", json!({"name": format!("Layer {layer}")}))
                .unwrap();
            let t = s as f64;
            let x = cx + 120.0 * (t / 3.0).cos();
            let y = cy + 120.0 * (t / 3.0).sin();
            driver
                .session_mut()
                .execute(
                    "paint.stroke",
                    json!({"points": [[x, y, 1.0]], "size": spec.w / 32, "color": format!("#{:02x}{:02x}{:02x}", 200 - (s*7) % 200, 40, 80)}),
                )
                .unwrap();
        }
        driver
            .session_mut()
            .execute("layer.setProps", json!({"name": "Layer 0", "opacity": 0.5}))
            .unwrap();
        driver
            .session_mut()
            .execute("filter.blur.gaussianBlur", json!({"radius": 2}))
            .unwrap();
        driver
            .session_mut()
            .execute("select.rect", json!({"x": spec.w/8, "y": spec.h/8, "width": spec.w/4, "height": spec.h/4}))
            .unwrap();

        // 4. Measure both unsaved-carry modes.
        let meta: PhotocraftMeta = driver.meta().unwrap();
        let document_bytes = std::fs::read(&doc_path).unwrap();
        let snapshot_bytes = driver.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
        let (journal_bytes, journal_steps) = {
            let b = driver.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
            let steps = driver.journal_steps();
            (b, steps.len())
        };
        let meta_bytes = serde_json::to_vec(&meta).unwrap();

        // 5. Replay oracle: a second clean session on the saved document
        //    replays the journal; the replayed revision must match and the
        //    replayed dirty snapshot must hash equal to the source's.
        let mut dest = InProcessDriver::new(InProcessConfig { work_root: work.clone() }).unwrap();
        dest.session_mut()
            .execute("file.open", json!({"path": doc_path.to_string_lossy()}))
            .unwrap();
        let steps: Vec<Step> = {
            let pairs: Vec<(String, Value)> = serde_json::from_slice(&journal_bytes).unwrap();
            pairs
                .into_iter()
                .map(|(id, params)| Step { id, params })
                .collect()
        };
        // The destination journal slice should start empty for replay; layers
        // are selected by NAME (portable), so replay is document-safe.
        let outcome = dest.apply_steps(&steps).unwrap();
        let dest_meta = dest.meta().unwrap();
        let replay_ok_revision = dest_meta.revision == meta.revision;
        let replay_snapshot = dest.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
        let replay_hash = hex_sha256(&replay_snapshot);
        let source_hash = hex_sha256(&snapshot_bytes);
        let replay_ok_hash = replay_hash == source_hash;
        println!(
            "  document {} B | snapshot(unsaved) {} B | journal(unsaved) {} B ({} steps) | meta {} B",
            document_bytes.len(), snapshot_bytes.len(), journal_bytes.len(), journal_steps, meta_bytes.len()
        );
        println!(
            "  replay: steps_ran={} revision match={} hash match={} ({src} vs {dst})",
            outcome.steps_ran, replay_ok_revision, replay_ok_hash,
            src = &source_hash[..12],
            dst = &replay_hash[..12]
        );

        let bfa_snapshot = document_bytes.len() + snapshot_bytes.len() + meta_bytes.len();
        let bfa_journal = document_bytes.len() + journal_bytes.len() + meta_bytes.len();
        results.push(json!({
            "spec": {
                "name": spec.name, "w": spec.w, "h": spec.h,
                "layers": spec.layers, "adjustments": spec.adjustments, "strokes": spec.strokes,
            },
            "document_bytes": document_bytes.len(),
            "unsaved_snapshot_bytes": snapshot_bytes.len(),
            "unsaved_journal_bytes": journal_bytes.len(),
            "journal_steps": journal_steps,
            "meta_bytes": meta_bytes.len(),
            "bytes_before_first_action_snapshot_mode": bfa_snapshot,
            "bytes_before_first_action_journal_mode": bfa_journal,
            "ratio_journal_over_snapshot_unsaved": journal_bytes.len() as f64 / snapshot_bytes.len().max(1) as f64,
            "replay_oracle": {
                "steps_ran": outcome.steps_ran,
                "revision_match": replay_ok_revision,
                "dirty_state_hash_match": replay_ok_hash,
                "source_dirty_sha256": source_hash,
                "replayed_dirty_sha256": replay_hash,
            },
            "source_meta": meta,
        }));
        let _ = std::fs::remove_file(&doc_path);
    }

    let evidence = json!({
        "experiment": "journal-vs-snapshot bytes_before_first_action",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "scope": "LOCAL (in-process engine sessions, one machine)",
        "authorize": "fail-closed authorize_policy on every session (incl. replay)",
        "prerequisites": "document + unsaved (mode-dependent) + meta; ephemeral workspace excluded (optional, deferred by progressive carry)",
        "results": results,
    });
    std::fs::write(&out_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {out_path}");
}
