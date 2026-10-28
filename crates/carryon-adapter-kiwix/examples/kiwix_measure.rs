//! kiwix_measure — the flagship byte-math measurement (TARGETS #4 shape) over
//! the REAL captured Kiwix session.
//!
//! ```sh
//! kiwix_measure <path-to.zim> [article_path]
//! ```
//!
//! Two independent strategy executions, same as the engine's definitions:
//!
//! - **Strategy A (full up front)**: transfer EVERY declared object — all
//!   prerequisites AND all optional (including the 27 MB ZIM bulk) — before
//!   the first action. `bytes_before_first_action` = sum of all objects'
//!   bytes actually moved through `read_object` chunk reads.
//! - **Strategy D (progressive)**: transfer only the `reader.restore`
//!   prerequisites (session + article + meta); the optional bulk stays
//!   behind. `bytes_before_first_action` = those bytes actually moved.
//!
//! Then the destination-side reconstruction check: from the D-strategy bytes
//! ALONE (no ZIM), rebuild the article and verify its sha256 against the
//! manifest's declared digest — "first paint from the article object alone".
//!
//! Label: LOCAL single-machine measurement of REAL object bytes (chunk-served
//! through the adapter). Not a network transfer; the physical LAN carry is
//! Phase 3. Numbers below are printed as measured.

use carryon_adapter_api::*;
use carryon_adapter_kiwix::KiwixAdapter;

const CHUNK: u64 = 64 * 1024;

/// Move the object's bytes chunk-by-chunk through `read_object` — the exact
/// serving path a real transfer uses — and return (bytes_moved, digest).
fn move_object(
    adapter: &KiwixAdapter,
    token: &SnapshotToken,
    object_id: &str,
    size: u64,
) -> Result<(u64, String), AdapterError> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut moved = 0u64;
    let mut off = 0u64;
    loop {
        let part = adapter.read_object(token, object_id, off, CHUNK)?;
        if part.is_empty() {
            break;
        }
        h.update(&part);
        moved += part.len() as u64;
        off += part.len() as u64;
    }
    assert_eq!(moved, size, "{object_id}: moved != declared size");
    let mut hex = String::new();
    for b in h.finalize() {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok((moved, hex))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let zim_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: kiwix_measure <path-to.zim> [article_path]");
            std::process::exit(2);
        }
    };
    let article = args.next().unwrap_or_else(|| "index".into());

    // The source adapter over the real ZIM, at the captured reading position
    // (scrollY 0 — the honest captured value).
    let mut adapter = KiwixAdapter::from_zim("kiwix-measure", &zim_path, &article, 0)
        .expect("open real zim session");

    let gen = adapter.current_generation("s").expect("gen");
    let token = adapter.begin_snapshot("s", gen).expect("begin snapshot");
    let manifest = adapter.describe_snapshot(&token).expect("manifest");
    let cut = CutRef {
        session: "kiwix-measure".into(),
        cut_number: 1,
    };
    let plan = adapter
        .resolve_action(
            &cut,
            &ActionRequestWire {
                class: "reader.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .expect("restore plan");

    let prereq: std::collections::HashSet<String> = plan
        .prerequisites
        .iter()
        .map(|v| v.object_id.clone())
        .collect();
    let optional: std::collections::HashSet<String> =
        plan.optional.iter().map(|v| v.object_id.clone()).collect();

    // ---- Strategy A: move EVERYTHING up front -----------------------------
    let mut a_bytes = 0u64;
    let mut a_detail = Vec::new();
    let mut article_hash_a = String::new();
    for o in &manifest.objects {
        if o.logical_size == 0 {
            continue; // nothing declared (e.g. bookmarks on a fresh capture)
        }
        let (moved, digest) =
            move_object(&adapter, &token, &o.object_id, o.logical_size).expect("strategy A move");
        a_bytes += moved;
        a_detail.push(format!("{:<24} {:>12}", o.object_id, moved));
        if o.object_id == "kiwix.article.v1" {
            article_hash_a = digest;
        }
    }
    // The ZIM bulk travels in strategy A even when it is NOT a manifest
    // object (real libraries exceed the core's 16 MiB entry cap — it is an L1
    // object; see all_entries). Move it through the same chunk path.
    let bulk_size = adapter.zim_bulk_size();
    let bulk_in_manifest = manifest
        .objects
        .iter()
        .any(|o| o.object_id == "kiwix.zimfile.v1");
    if !bulk_in_manifest && bulk_size > 0 {
        let (moved, digest) =
            move_object(&adapter, &token, "kiwix.zimfile.v1", bulk_size)
                .expect("strategy A bulk move");
        a_bytes += moved;
        a_detail.push(format!(
            "{:<24} {:>12}  sha256={digest}",
            "kiwix.zimfile.v1", moved
        ));
    }

    // ---- Strategy D: move prerequisites only -----------------------------
    let mut d_bytes = 0u64;
    let mut d_detail = Vec::new();
    let mut d_article_bytes = Vec::new();
    for o in &manifest.objects {
        if !prereq.contains(&o.object_id) || o.logical_size == 0 {
            continue;
        }
        let (moved, digest) =
            move_object(&adapter, &token, &o.object_id, o.logical_size).expect("strategy D move");
        d_bytes += moved;
        d_detail.push(format!(
            "{:<24} {:>12}  sha256={digest}",
            o.object_id, moved
        ));
        if o.object_id == "kiwix.article.v1" {
            assert_eq!(digest, o.content_hash, "article digest matches manifest");
            d_article_bytes = adapter
                .read_object(&token, "kiwix.article.v1", 0, o.logical_size)
                .expect("article bytes");
        }
    }
    let mut deferred: Vec<String> = manifest
        .objects
        .iter()
        .filter(|o| optional.contains(&o.object_id) && o.logical_size > 0)
        .map(|o| format!("{} ({} B)", o.object_id, o.logical_size))
        .collect();
    // The bulk is deferred in D even when it is not a manifest object.
    if !bulk_in_manifest && bulk_size > 0 {
        deferred.insert(0, format!("kiwix.zimfile.v1 ({bulk_size} B, L1 object)"));
    }

    // ---- Destination-side reconstruction from D bytes ALONE ----------------
    // No ZIM: the article object alone must reproduce the manifest digest.
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&d_article_bytes);
    let mut rebuilt = String::new();
    for b in h.finalize() {
        rebuilt.push_str(&format!("{b:02x}"));
    }
    let declared = manifest
        .objects
        .iter()
        .find(|o| o.object_id == "kiwix.article.v1")
        .expect("article entry")
        .content_hash
        .clone();
    let first_paint_ok = rebuilt == declared && article_hash_a == declared;

    // ---- Report ------------------------------------------------------------
    println!("=== kiwix_measure (LOCAL, real object bytes through read_object) ===");
    println!("zim = {zim_path}");
    println!("article = {article}");
    println!();
    println!("strategy A (everything up front):");
    for l in &a_detail {
        println!("   {l}");
    }
    println!("   bytes_before_first_action = {a_bytes}");
    println!();
    println!("strategy D (prerequisites only):");
    for l in &d_detail {
        println!("   {l}");
    }
    println!("   bytes_before_first_action = {d_bytes}");
    println!("   deferred (optional, on demand): {}", deferred.join(", "));
    println!();
    let ratio = d_bytes as f64 / a_bytes as f64;
    println!("D/A ratio            = {ratio:.6} ({:.4}%)", ratio * 100.0);
    println!("D < 50% of A         = {}", d_bytes * 2 < a_bytes);
    println!("first paint from article object alone (no ZIM) = {first_paint_ok}");
    let factor = a_bytes as f64 / d_bytes as f64;
    println!("A/D asymmetry factor = {factor:.1}x");
    if !first_paint_ok {
        std::process::exit(1);
    }
    println!("MEASURE_OK");
}
