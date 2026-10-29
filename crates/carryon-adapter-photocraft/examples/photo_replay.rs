//! Photo-document journal replay (Phase 3, task 1): closes the Phase-2 gap —
//! the size probe measured the real-photo case but did not REPLAY it.
//!
//! Builds a layered document from a real photo (2400×1500
//! `docs/images/photocraft-demo.jpg`), saves it (journal checkpoint), applies
//! a multi-step realistic edit script through the fail-closed gate, measures
//! both unsaved-carry modes, then REPLAYS the journal on a second clean
//! session opened on the saved file. The composite-pixel hash of the replayed
//! document must equal the source's (the Phase-2 oracle).
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --features inprocess \
//!   --example photo_replay -- <photo.jpg> <out.json>
//! ```

use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{hex_sha256, UnsavedMode};
use carryon_adapter_photocraft::{PhotocraftDriver, Step};
use serde_json::{json, Value};
use std::path::PathBuf;

fn main() {
    let photo = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: photo_replay <photo.jpg> <out.json>"),
    );
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "docs/artcraft-bridge/evidence/photo-replay.json".into());
    let work = std::env::temp_dir().join("pcraft-photo-replay");
    std::fs::create_dir_all(&work).unwrap();
    let photo_in_root = work.join(photo.file_name().unwrap());
    std::fs::copy(&photo, &photo_in_root).unwrap();

    let mut src = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    let rel = photo.file_name().unwrap().to_string_lossy().into_owned();
    src.open_under_root(&rel).unwrap();
    // Layered document: photo + adjustment + a retouch layer.
    src.session_mut()
        .execute("image.duplicate", json!({}))
        .unwrap();
    src.session_mut()
        .execute(
            "layer.newAdjustmentLayer.brightnessContrast",
            json!({"brightness": 12, "contrast": 8}),
        )
        .unwrap();
    src.session_mut()
        .execute("layer.new.layer", json!({"name": "Retouch"}))
        .unwrap();
    src.session_mut()
        .execute("layer.select", json!({"name": "Retouch"}))
        .unwrap();
    let saved = src.save_active_under_root("photo-layered.pcraft").unwrap();

    // The multi-step post-save edit script (what the journal carries).
    src.session_mut()
        .execute(
            "paint.stroke",
            json!({"points": [[700, 480, 1.0], [820, 520, 0.8]], "size": 220, "hardness": 0.3, "color": "#1a2b4c"}),
        )
        .unwrap();
    src.session_mut()
        .execute(
            "paint.stroke",
            json!({"points": [[1250, 880, 1.0], [1350, 900, 0.9], [1450, 940, 0.6]], "size": 160, "hardness": 0.5, "color": "#c33"}),
        )
        .unwrap();
    src.session_mut()
        .execute("layer.setProps", json!({"name": "Retouch", "opacity": 0.7}))
        .unwrap();
    src.session_mut()
        .execute(
            "select.rect",
            json!({"x": 400, "y": 300, "width": 900, "height": 600}),
        )
        .unwrap();
    src.session_mut()
        .execute("filter.blur.gaussianBlur", json!({"radius": 1.5}))
        .unwrap();
    src.session_mut()
        .execute("select.deselect", json!({}))
        .unwrap();

    // Measure both carry modes + the composite oracle basis.
    let meta = src.meta().unwrap();
    assert!(meta.dirty);
    let document_bytes = std::fs::read(&saved).unwrap();
    let snapshot_bytes = src.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
    let journal_bytes = src.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
    let steps: Vec<Step> = {
        let pairs: Vec<(String, Value)> = serde_json::from_slice(&journal_bytes).unwrap();
        pairs
            .into_iter()
            .map(|(id, params)| Step { id, params })
            .collect()
    };
    let source_pixels = src.composite_bytes_of_active().unwrap();

    // Replay on a second clean session.
    let mut dst = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    dst.open_under_root("photo-layered.pcraft").unwrap();
    let outcome = dst.apply_steps(&steps).unwrap();
    let dst_meta = dst.meta().unwrap();
    let replay_pixels = dst.composite_bytes_of_active().unwrap();
    let hash_src = hex_sha256(&source_pixels);
    let hash_dst = hex_sha256(&replay_pixels);
    let replay_ok = hash_src == hash_dst;

    println!(
        "document {} B | snapshot(unsaved) {} B | journal(unsaved) {} B ({} steps) | composite {} B",
        document_bytes.len(),
        snapshot_bytes.len(),
        journal_bytes.len(),
        steps.len(),
        source_pixels.len()
    );
    println!(
        "replay: steps_ran={} (of {}) pixel-hash {} ({src} vs {dst})",
        outcome.steps_ran,
        steps.len(),
        if replay_ok { "MATCH" } else { "MISMATCH" },
        src = &hash_src[..12],
        dst = &hash_dst[..12]
    );

    let evidence = json!({
        "experiment": "photo-document journal replay (real photo)",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "scope": "LOCAL (in-process engine sessions, one machine)",
        "source_photo": {
            "name": photo.file_name().unwrap().to_string_lossy(),
            "bytes": std::fs::metadata(&photo).unwrap().len(),
            "dimensions": "2400x1500",
        },
        "measurements": {
            "document_bytes": document_bytes.len(),
            "unsaved_snapshot_bytes": snapshot_bytes.len(),
            "unsaved_journal_bytes": journal_bytes.len(),
            "journal_steps": steps.len(),
            "journal_step_ids": steps.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            "composite_png_bytes": source_pixels.len(),
        },
        "replay_oracle": {
            "steps_ran": outcome.steps_ran,
            "steps_expected": steps.len(),
            "revision_source_session_relative": meta.revision,
            "revision_dest_session_relative": dst_meta.revision,
            "composite_pixel_sha256_source": hash_src,
            "composite_pixel_sha256_replayed": hash_dst,
            "composite_pixel_hash_match": replay_ok,
            "note": "same caveat as Phase 2: .pcraft bytes and revision counts are session-relative; the composite-pixel hash is the oracle",
        },
    });
    std::fs::write(&out_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {out_path}");
    assert!(
        replay_ok,
        "photo journal replay must reproduce the composite pixels"
    );
    assert_eq!(outcome.steps_ran, steps.len());
    println!("PHOTO REPLAY ORACLE: MATCH");
}
