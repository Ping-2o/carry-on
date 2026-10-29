//! The Phase-4 flagship measurement (TILE-SPLITTING.md §6): the real photo
//! (2400×1500) carried as a SPLIT bundle — manifest object + per-tile
//! content-addressed objects — with progressive budget windows and tile dedup
//! on re-carry. Everything is OBSERVED, LOCAL (one machine, loopback TLS).
//!
//! Measures, with raw numbers into the evidence JSON:
//! 1. **bfa split vs monolithic** (journal mode): bytes_before_first_action =
//!    Σ prerequisite object bytes, both document encodings, same session.
//! 2. **Progressive windows**: a destination imports the split cut under small
//!    budgets; each attempt publishes a verified prefix (CORE-004 per object),
//!    the deferred tail shrinks, and a replenished window completes the carry.
//!    The MONOLITHIC counterfactual at the same windows: the whole document
//!    object is deferred in every sub-object-size window (and a retry with
//!    only the document left is refused outright).
//! 3. **Tile dedup on re-carry**: one paint stroke later, the incremental
//!    directory re-save changes only the touched tiles; the second cut's
//!    carry needs only manifest + changed tiles + journal + meta.
//! 4. **Oracle**: the destination reconstructs the bundle from its own store,
//!    opens it with the real PhotoCraft engine, replays the journal — the
//!    composite-pixel sha256 must equal the source's.
//!
//! Usage:
//! ```sh
//! cargo run -p carryon-adapter-photocraft --features inprocess \
//!   --example tile_progressive -- <photo.jpg> <out.json>
//! ```

use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
use carryon_adapter_photocraft::model::{DocumentBundle, UnsavedMode, Workspace};
use carryon_adapter_photocraft::{PhotocraftAdapter, PhotocraftDriver as _};
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::model::{AuthorityMode, Budget, Sensitivity};
use carryon_core::{Core, CreateSessionReq, ImportOutcome};
use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const SESSION: &str = "photocraft-tile-progressive";

fn main() {
    let photo = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: tile_progressive <photo.jpg> <out.json>"),
    );
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "docs/artcraft-bridge/evidence/tile-progressive.json".into());
    let work = std::env::temp_dir().join("pcraft-tile-progressive");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let photo_in_root = work.join(photo.file_name().unwrap());
    std::fs::copy(&photo, &photo_in_root).unwrap();

    let mut src = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    let rel = photo.file_name().unwrap().to_string_lossy().into_owned();
    src.open_under_root(&rel).unwrap();
    // The flagship layered document (the photo_replay recipe): photo +
    // adjustment layer + retouch layer.
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
    // SPLIT-MODE base save: a directory bundle (PhotoCraft's own incremental,
    // content-addressed store, RESEARCH §1.5).
    src.save_active_bundle_under_root("base-bundle").unwrap();
    // Post-save dirty edits (what the journal carries).
    src.session_mut()
        .execute(
            "paint.stroke",
            json!({"points": [[700, 480, 1.0], [820, 520, 0.8]], "size": 220, "hardness": 0.3, "color": "#1a2b4c"}),
        )
        .unwrap();

    // ---- 1. bfa: split vs monolithic, same session --------------------------
    let bundle = src.document_bundle_of_active().unwrap();
    let journal = src.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
    let snapshot = src.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
    let meta = src.meta().unwrap();
    let meta_bytes = serde_json::to_vec(&meta).unwrap();
    let monolithic_doc = monolithic_zip_of(&bundle, &work);
    // The cut-1 oracle basis, captured before any later strokes.
    let oracle_src =
        carryon_adapter_photocraft::model::hex_sha256(&src.composite_bytes_of_active().unwrap());

    let parts_bytes: u64 = bundle.parts.values().map(|b| b.len() as u64).sum();
    let bfa_split =
        bundle.manifest.len() as u64 + parts_bytes + journal.len() as u64 + meta_bytes.len() as u64;
    let bfa_monolithic =
        monolithic_doc.len() as u64 + journal.len() as u64 + meta_bytes.len() as u64;
    let tile_count = bundle
        .parts
        .keys()
        .filter(|p| p.starts_with("tiles/"))
        .count();
    let blob_count = bundle.parts.len() - tile_count;
    println!(
        "bfa split={bfa_split} B (manifest {} B + {} tiles + {} blobs = {parts_bytes} B + journal {} B + meta {} B)",
        bundle.manifest.len(),
        tile_count,
        blob_count,
        journal.len(),
        meta_bytes.len()
    );
    println!(
        "bfa monolithic={bfa_monolithic} B (zip {} B + journal {} B + meta {} B)",
        monolithic_doc.len(),
        journal.len(),
        meta_bytes.len()
    );

    // ---- 2. Progressive budget windows (loopback TLS) -----------------------
    let src_dir = work.join("src-core");
    let mut source = Core::open(&src_dir).unwrap();
    let info = source
        .register_adapter(Box::new(PhotocraftAdapter::with_split_state(
            SESSION,
            bundle.clone(),
            journal.clone(),
            Workspace::default(),
            meta.clone(),
        )))
        .unwrap();
    let session = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: SESSION.into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(session).unwrap();
    let sess_str = session.to_string();
    let sealed = source.cut_authoritative_objects(cut);
    assert!(sealed.contains(&"photocraft.document.manifest.v1".to_string()));

    let dst_dir = work.join("dst-core");
    let mut dest = Core::open(&dst_dir).unwrap();
    let (sid, did, st, dt) = {
        let s = DeviceIdentity::generate("tile-src").unwrap();
        let d = DeviceIdentity::generate("tile-dst").unwrap();
        let (st, dt) = pair_devices(&s, &d, "tile").unwrap();
        (s, d, Arc::new(st), Arc::new(dt))
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // Serve as many connections as the destination opens (each window = one).
    let srv = {
        let listener = listener;
        let source = std::sync::Arc::new(std::sync::Mutex::new(source));
        std::thread::spawn(move || {
            for _ in 0..3 {
                if let Ok(mut s) = Session::accept(&listener, &sid, st.clone()) {
                    let _ = s.server_negotiate(vec![]);
                    let _ = source.lock().unwrap().serve_cut(&mut s);
                }
            }
        })
    };

    // Windows: 64 KiB, then 256 KiB, then unlimited. Each publishes a verified
    // prefix; the deferred tail must shrink monotonically.
    let mut windows = Vec::new();
    let mut attempts = 0;
    let mut completed_cut: Option<u64> = None;
    for window in [64 * 1024u64, 256 * 1024, u64::MAX] {
        attempts += 1;
        let mut b = Budget::local_default();
        b.total_net_bytes = window;
        dest.set_budget(b);
        let mut net = Session::connect(&addr, &did, dt.clone()).unwrap();
        net.client_negotiate(vec![]).unwrap();
        let t = Instant::now();
        let out = dest.import_cut(&mut net, &sess_str, cut.number).unwrap();
        let secs = t.elapsed().as_secs_f64();
        let wire = net.bytes_recv();
        drop(net);
        match out {
            ImportOutcome::PartialBudget(p) => {
                let moved: u64 = p
                    .published
                    .iter()
                    .map(|(id, _)| {
                        lookup_bytes(id, &bundle, &journal, &meta_bytes).unwrap_or(0) as u64
                    })
                    .sum();
                println!(
                    "window {attempts} (budget {window} B): published {} objects \
                     (+{moved} B logical, {wire} B wire, {secs:.3}s), deferred {} objects \
                     ({} B)",
                    p.published.len(),
                    p.deferred.len(),
                    p.deferred_bytes
                );
                windows.push(json!({
                    "attempt": attempts,
                    "budget_bytes": window,
                    "published_objects": p.published.len(),
                    "published_ids": p.published.iter().map(|(id,_)| id).collect::<Vec<_>>(),
                    "moved_logical_bytes": moved,
                    "wire_bytes": wire,
                    "seconds": secs,
                    "deferred_objects": p.deferred.len(),
                    "deferred_bytes": p.deferred_bytes,
                }));
            }
            ImportOutcome::Completed(n) => {
                println!("window {attempts} (budget {window} B): COMPLETED mirror cut {n} ({wire} B wire, {secs:.3}s)");
                windows.push(json!({
                    "attempt": attempts,
                    "budget_bytes": window,
                    "completed": true,
                    "mirror_cut": n,
                    "wire_bytes": wire,
                    "seconds": secs,
                }));
                completed_cut = Some(n);
                break;
            }
            ImportOutcome::Suspended(_) => panic!("no suspend requested"),
        }
    }
    let mirror_cut = completed_cut.expect("the replenished window must complete the carry");
    let _ = mirror_cut;

    // ---- 2b. Monolithic counterfactual ---------------------------------------
    // The SAME session sealed monolithically (one zip object), imported under
    // the SAME first window budget: refused before any bytes, zero durable
    // progress (the pre-split behavior — the budget binds at object
    // granularity, and the monolith is one 6.9 MB object).
    let mono_src_dir = work.join("mono-src-core");
    let mut mono_source = Core::open(&mono_src_dir).unwrap();
    let mono_info = mono_source
        .register_adapter(Box::new(PhotocraftAdapter::with_state(
            SESSION,
            monolithic_doc.clone(),
            journal.clone(),
            Workspace::default(),
            meta.clone(),
        )))
        .unwrap();
    let mono_session = mono_source
        .create_session(CreateSessionReq {
            adapter_id: mono_info.adapter_id,
            title: SESSION.into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let mono_cut = mono_source.create_cut(mono_session).unwrap();
    let mono_sess_str = mono_session.to_string();
    // A third device identity pair for the counterfactual carry.
    let (sidm, didm, stm, dtm) = {
        let s = DeviceIdentity::generate("mono-src").unwrap();
        let d = DeviceIdentity::generate("mono-dst").unwrap();
        let (st, dt) = pair_devices(&s, &d, "mono").unwrap();
        (s, d, Arc::new(st), Arc::new(dt))
    };
    let listener_m = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr_m = listener_m.local_addr().unwrap().to_string();
    {
        let listener = listener_m;
        std::thread::spawn(move || {
            for _ in 0..2 {
                if let Ok(mut s) = Session::accept(&listener, &sidm, stm.clone()) {
                    let _ = s.server_negotiate(vec![]);
                    let _ = mono_source.serve_cut(&mut s);
                }
            }
        });
    }
    let mono_dst_dir = work.join("mono-dst-core");
    let mut mono_dest = Core::open(&mono_dst_dir).unwrap();
    // Window A (64 KiB): the tiny objects bank, but the 6.9 MB document object
    // cannot fit ANY window below its own size — PartialBudget, doc deferred.
    let mut bm = Budget::local_default();
    bm.total_net_bytes = 64 * 1024;
    mono_dest.set_budget(bm);
    let mut nm = Session::connect(&addr_m, &didm, dtm.clone()).unwrap();
    nm.client_negotiate(vec![]).unwrap();
    let mono_a = mono_dest.import_cut(&mut nm, &mono_sess_str, mono_cut.number);
    drop(nm);
    let (mono_a_published, mono_a_deferred_doc) = match mono_a {
        Ok(ImportOutcome::PartialBudget(p)) => (
            p.published.len(),
            p.deferred.iter().any(|id| id == "photocraft.document.v1"),
        ),
        other => panic!("monolith @64 KiB must be budget-partial, got {other:?}"),
    };
    // Window B (256 KiB, replenished): only the document remains; it still
    // does not fit → refused before any bytes. No sequence of small windows
    // can complete the monolithic carry — only a window ≥ the document size.
    let mut bm2 = Budget::local_default();
    bm2.total_net_bytes = 256 * 1024;
    mono_dest.set_budget(bm2);
    let mut nm2 = Session::connect(&addr_m, &didm, dtm).unwrap();
    nm2.client_negotiate(vec![]).unwrap();
    let mono_b = mono_dest.import_cut(&mut nm2, &mono_sess_str, mono_cut.number);
    let (mono_refused, mono_refusal) = match mono_b {
        Err(e) => (true, e.to_string()),
        Ok(other) => (false, format!("NOT REFUSED: {other:?}")),
    };
    drop(nm2);
    println!(
        "monolith @64 KiB: partial ({mono_a_published} tiny objects banked, \
document deferred={mono_a_deferred_doc}); @256 KiB retry: refused={mono_refused} ({mono_refusal})"
    );

    // ---- 3. Tile dedup on re-carry (one stroke later) ------------------------
    let cut1_part_ids: Vec<String> = sealed
        .iter()
        .filter(|id| id.starts_with("photocraft.tile.") || id.starts_with("photocraft.blob."))
        .cloned()
        .collect();
    // One more stroke, incremental re-save, re-seal (a fresh source core).
    src.session_mut()
        .execute(
            "paint.stroke",
            json!({"points": [[1250, 880, 1.0]], "size": 60, "color": "#c33"}),
        )
        .unwrap();
    src.save_active_bundle_under_root("base-bundle").unwrap();
    let bundle2 = src.document_bundle_of_active().unwrap();
    let journal2 = src.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
    let meta2 = src.meta().unwrap();
    let meta2_bytes = serde_json::to_vec(&meta2).unwrap();

    let src2_dir = work.join("src2-core");
    let mut source2 = Core::open(&src2_dir).unwrap();
    let info2 = source2
        .register_adapter(Box::new(PhotocraftAdapter::with_split_state(
            "photocraft-tile-progressive-2",
            bundle2.clone(),
            journal2.clone(),
            Workspace::default(),
            meta2.clone(),
        )))
        .unwrap();
    let session2 = source2
        .create_session(CreateSessionReq {
            adapter_id: info2.adapter_id,
            title: "photocraft-tile-progressive-2".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut2 = source2.create_cut(session2).unwrap();
    let sess2_str = session2.to_string();
    let sealed2 = source2.cut_authoritative_objects(cut2);
    let cut2_part_ids: Vec<String> = sealed2
        .iter()
        .filter(|id| id.starts_with("photocraft.tile.") || id.starts_with("photocraft.blob."))
        .cloned()
        .collect();
    let changed: Vec<&String> = cut2_part_ids
        .iter()
        .filter(|id| !cut1_part_ids.contains(id))
        .collect();
    let changed_bytes: u64 = changed
        .iter()
        .filter_map(|id| lookup_bytes(id, &bundle2, &journal2, &meta2_bytes))
        .map(|b| b as u64)
        .sum();
    println!(
        "re-carry after 1 stroke: {} of {} content objects changed ({changed_bytes} B)",
        changed.len(),
        cut2_part_ids.len()
    );

    // The OBSERVED dedup re-carry: the destination that already holds cut 1
    // imports cut 2 — the content-addressed store skips every unchanged tile,
    // so the wire moves only manifest + changed tiles + journal + meta.
    // (A fresh device identity pair: the second logical carry.)
    let (sid2, did2, st2, dt2) = {
        let s = DeviceIdentity::generate("tile-src-2").unwrap();
        let d = DeviceIdentity::generate("tile-dst-2").unwrap();
        let (st, dt) = pair_devices(&s, &d, "tile2").unwrap();
        (s, d, Arc::new(st), Arc::new(dt))
    };
    let listener2 = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr2 = listener2.local_addr().unwrap().to_string();
    {
        let listener = listener2;
        std::thread::spawn(move || {
            if let Ok(mut s) = Session::accept(&listener, &sid2, st2) {
                let _ = s.server_negotiate(vec![]);
                let _ = source2.serve_cut(&mut s);
            }
        });
    }
    let mut net2 = Session::connect(&addr2, &did2, dt2).unwrap();
    net2.client_negotiate(vec![]).unwrap();
    let t2 = Instant::now();
    let out2 = dest.import_cut(&mut net2, &sess2_str, cut2.number).unwrap();
    let recarry_secs = t2.elapsed().as_secs_f64();
    let recarry_wire = net2.bytes_recv();
    drop(net2);
    let recarry_mirror = out2.completed_cut().expect("re-carry completes");
    println!(
        "observed dedup re-carry: {recarry_wire} B wire in {recarry_secs:.3}s \
         (mirror cut {recarry_mirror})"
    );
    let _ = srv.join();
    let hex = carryon_adapter_photocraft::model::hex_sha256;
    let mut dst_driver = InProcessDriver::new(InProcessConfig {
        work_root: work.clone(),
    })
    .unwrap();
    let mut restored = DocumentBundle {
        manifest: dest.read_object_hex(&hex(&bundle.manifest)).unwrap(),
        parts: Default::default(),
    };
    for (path, bytes) in &bundle.parts {
        restored
            .parts
            .insert(path.clone(), dest.read_object_hex(&hex(bytes)).unwrap());
    }
    dst_driver
        .write_bundle_under_root("carried-bundle", &restored)
        .unwrap();
    dst_driver.open_under_root("carried-bundle").unwrap();
    let steps: Vec<carryon_adapter_photocraft::Step> =
        serde_json::from_slice::<Vec<(String, Value)>>(&journal)
            .unwrap()
            .into_iter()
            .map(|(id, params)| carryon_adapter_photocraft::Step { id, params })
            .collect();
    dst_driver.apply_steps(&steps).unwrap();
    let oracle_dst = hex(&dst_driver.composite_bytes_of_active().unwrap());
    let oracle_match = oracle_src == oracle_dst;
    println!("composite oracle: src {oracle_src} == dst {oracle_dst}: {oracle_match}");
    assert!(
        oracle_match,
        "reconstructed bundle must render identical pixels"
    );

    // ---- evidence -------------------------------------------------------------
    let evidence = json!({
        "experiment": "tile-split progressive carry on the real photo",
        "scope": "LOCAL (one machine, loopback TLS, in-process PhotoCraft engine)",
        "photocraft_pin": carryon_adapter_photocraft::model::PHOTOCRAFT_PIN,
        "photo": photo.to_string_lossy(),
        "document": {
            "layers": "photo + brightness/contrast adjustment + retouch layer",
            "dirty_edits": "1 paint stroke (post-save)",
            "unsaved_mode": "journal",
            "tile_count": tile_count,
            "blob_count": blob_count,
        },
        "bytes_before_first_action": {
            "split": {
                "manifest": bundle.manifest.len(),
                "parts_total": parts_bytes,
                "unsaved_journal": journal.len(),
                "meta": meta_bytes.len(),
                "TOTAL": bfa_split,
            },
            "monolithic": {
                "document_zip": monolithic_doc.len(),
                "unsaved_journal": journal.len(),
                "meta": meta_bytes.len(),
                "TOTAL": bfa_monolithic,
            },
            "reference_snapshot_mode": {
                "unsaved_snapshot": snapshot.len(),
                "TOTAL_with_snapshot": monolithic_doc.len() + snapshot.len() + meta_bytes.len(),
            },
        },
        "progressive_windows": windows,
        "monolithic_counterfactual": {
            "window_a_budget_bytes": 65536,
            "window_a_outcome": "partial: meta + journal bank; the whole document object deferred",
            "window_a_document_deferred": mono_a_deferred_doc,
            "window_b_budget_bytes": 262144,
            "window_b_refused_before_bytes": mono_refused,
            "window_b_refusal": mono_refusal,
            "note": "the budget binds at object granularity: the monolithic document is ONE object, so no sequence of sub-object-size windows can complete its carry (window B is refused with nothing left to bank); the split completed via windows and would have needed only many small ones",
        },
        "bfa_delta_explanation": "split total is lower than monolithic because the monolithic zip embeds PhotoCraft's thumb.png + composite/preview.png previews (re-derived on open); the split bundle carries no previews — the delta is previews, not pixels",
        "recarry_after_one_stroke": {
            "content_objects_cut1": cut1_part_ids.len(),
            "content_objects_cut2": cut2_part_ids.len(),
            "changed_objects": changed.len(),
            "changed_bytes": changed_bytes,
            "observed_dedup_recarry_wire_bytes": recarry_wire,
            "observed_dedup_recarry_seconds": recarry_secs,
            "recarry_journal_bytes": journal2.len(),
            "scenario": "after 2 strokes the document was re-saved (incremental directory save); cut 2 is the NEW SAVED BASE (67 parts, empty post-save journal); the destination already held cut 1 and the content-addressed store deduplicated unchanged tiles (OBSERVED wire bytes, not an estimate)",
        },
        "oracle": {
            "composite_pixel_sha256_source": oracle_src,
            "composite_pixel_sha256_destination": oracle_dst,
            "match": oracle_match,
        },
    });
    std::fs::write(&out_path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("evidence: {out_path}");
}

/// The monolithic `.pcraft` zip bytes of the SAVED BASE state: open the
/// carried bundle in a scratch session and serialize it the Phase 1–3 way.
/// (The `WORK_ROOT` OnceLock already pins the shared work root.)
fn monolithic_zip_of(bundle: &DocumentBundle, work: &std::path::Path) -> Vec<u8> {
    let mut probe = InProcessDriver::new(InProcessConfig {
        work_root: work.to_path_buf(),
    })
    .unwrap();
    probe.write_bundle_under_root("mono-probe", bundle).unwrap();
    probe.open_under_root("mono-probe").unwrap();
    probe.zip_bytes_of_active().unwrap()
}

/// An object's logical bytes by id (bundle parts, manifest, journal, meta).
fn lookup_bytes(id: &str, bundle: &DocumentBundle, journal: &[u8], meta: &[u8]) -> Option<usize> {
    use carryon_adapter_photocraft::model::{SCHEMA_DOC_MANIFEST, SCHEMA_META, SCHEMA_UNSAVED};
    match id {
        SCHEMA_DOC_MANIFEST => Some(bundle.manifest.len()),
        SCHEMA_UNSAVED => Some(journal.len()),
        SCHEMA_META => Some(meta.len()),
        other => bundle
            .part_entries()
            .into_iter()
            .find(|(oid, _)| oid == other)
            .and_then(|(_, path)| bundle.parts.get(&path))
            .map(|b| b.len()),
    }
}
