//! 16 MiB object-cap decision probe (Phase 2, task 4): measure REAL `.pcraft`
//! sizes, feature `inprocess`.
//!
//! The Phase-1 skeleton noted the Carry-On core caps `max_object_bytes` at
//! 16 MiB (registration refuses anything larger — observed in a test run).
//! The Phase-2 measurement documents were synthetic sparse canvases (white
//! background + thin strokes), which understate real photo documents. This
//! probe builds documents from a REAL photo (the repo's 2400×1500
//! `docs/images/photocraft-demo.jpg`) and measures:
//!
//! 1. a single-layer photo document saved as `.pcraft`;
//! 2. a realistic layered edit (photo + duplicated layers + paint + an
//!    adjustment layer), saved clean and as a dirty snapshot;
//! 3. the journal-mode bytes for the same dirty state (for contrast).
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --features inprocess \
//!   --example pcraft_size_probe -- <photo.jpg> <out.json>
//! ```

use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{hex_sha256, UnsavedMode};
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let photo = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: pcraft_size_probe <photo.jpg> <out.json>"),
    );
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "docs/artcraft-bridge/evidence/pcraft-size-probe.json".into());
    let work = std::env::temp_dir().join("pcraft-cap-probe");
    std::fs::create_dir_all(&work).unwrap();
    // Copy the photo under the work root (the driver only opens under root).
    let photo_in_root = work.join(photo.file_name().unwrap());
    std::fs::copy(&photo, &photo_in_root).unwrap();

    let mut driver = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();

    // 1. Single-layer photo document.
    let rel = photo.file_name().unwrap().to_string_lossy().into_owned();
    driver.open_under_root(&rel).unwrap();
    let single_path = driver
        .save_active_under_root("photo-single.pcraft")
        .unwrap();
    let single_bytes = std::fs::read(&single_path).unwrap();

    // 2. Realistic layered edit: duplicate the photo twice, paint on one
    //    duplicate, add an adjustment layer, save clean.
    driver
        .session_mut()
        .execute("image.duplicate", json!({}))
        .unwrap();
    driver
        .session_mut()
        .execute(
            "layer.newAdjustmentLayer.brightnessContrast",
            json!({"brightness": 12, "contrast": 8}),
        )
        .unwrap();
    driver
        .session_mut()
        .execute("layer.new.layer", json!({"name": "Retouch"}))
        .unwrap();
    driver
        .session_mut()
        .execute("layer.select", json!({"name": "Retouch"}))
        .unwrap();
    driver
        .session_mut()
        .execute(
            "paint.stroke",
            json!({"points": [[800, 500, 1.0], [900, 520, 1.0], [1000, 560, 1.0]], "size": 250, "hardness": 0.2, "color": "#1a2b4c"}),
        )
        .unwrap();
    let layered_path = driver
        .save_active_under_root("photo-layered.pcraft")
        .unwrap();
    let layered_bytes = std::fs::read(&layered_path).unwrap();

    // 3. The same document, dirty again (a further edit), both carry modes.
    driver
        .session_mut()
        .execute(
            "paint.stroke",
            json!({"points": [[1200, 900, 1.0]], "size": 400, "hardness": 0.0, "color": "#ff3333"}),
        )
        .unwrap();
    let dirty_snapshot = driver.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
    let dirty_journal = driver.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
    let composite = driver.composite_bytes_of_active().unwrap();

    let evidence = json!({
        "experiment": "pcraft-size-probe (16 MiB core object cap decision evidence)",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "scope": "LOCAL (in-process engine session, one machine)",
        "source_photo": {
            "name": photo.file_name().unwrap().to_string_lossy(),
            "bytes": std::fs::metadata(&photo).unwrap().len(),
        },
        "core_cap_bytes": 16 * 1024 * 1024,
        "measurements": {
            "single_layer_photo_pcraft_bytes": single_bytes.len(),
            "layered_photo_pcraft_bytes": layered_bytes.len(),
            "dirty_state_snapshot_pcraft_bytes": dirty_snapshot.len(),
            "dirty_state_journal_bytes": dirty_journal.len(),
            "composite_png_bytes": composite.len(),
        },
        "single_layer_over_cap": single_bytes.len() > 16 * 1024 * 1024,
        "layered_over_cap": layered_bytes.len() > 16 * 1024 * 1024,
        "dirty_snapshot_over_cap": dirty_snapshot.len() > 16 * 1024 * 1024,
        "digests": {
            "single_layer_pcraft": hex_sha256(&single_bytes),
            "layered_pcraft": hex_sha256(&layered_bytes),
            "dirty_snapshot_pcraft": hex_sha256(&dirty_snapshot),
            "dirty_journal": hex_sha256(&dirty_journal),
            "composite_png": hex_sha256(&composite),
        },
    });
    std::fs::write(&out_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {out_path}");
    println!(
        "single-layer photo .pcraft: {} B | layered: {} B | dirty snapshot: {} B | journal: {} B (cap: 16 MiB)",
        single_bytes.len(),
        layered_bytes.len(),
        dirty_snapshot.len(),
        dirty_journal.len()
    );
}
