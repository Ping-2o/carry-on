//! Object schemas and state types for the PhotoCraft adapter
//! (`docs/artcraft-bridge/ADAPTER-DESIGN.md` §1).
//!
//! The taxonomy mirrors `carryon-adapter-editor`'s four-object session with the
//! divergences the PhotoCraft research pinned down
//! (`docs/artcraft-bridge/RESEARCH.md`):
//!
//! - the selection is **document state** in PhotoCraft (persisted in
//!   `.pcraft`, RESEARCH §1.2), so it travels inside the document/unsaved
//!   objects, not in the workspace object;
//! - history/undo snapshots are **not serializable** in PhotoCraft (RESEARCH
//!   §1.2) and are not carried;
//! - the unsaved-edits object has two carry modes: a `.pcraft` snapshot of the
//!   dirty state (any driver) or a command journal `[[id, params], …]` from the
//!   last saved revision (in-process driver only, RESEARCH §1.3).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Saved document as single-file `.pcraft` bytes (Authoritative).
pub const SCHEMA_DOC: &str = "photocraft.document.v1";
/// Unsaved edits (Authoritative, parents = document).
pub const SCHEMA_UNSAVED: &str = "photocraft.unsaved_edits.v1";
/// Identity/provenance meta (Authoritative).
pub const SCHEMA_META: &str = "photocraft.meta.v1";
/// Ephemeral working position (Ephemeral/optional).
pub const SCHEMA_WORKSPACE: &str = "photocraft.workspace.v1";

/// The saved document as a **split bundle** (TILE-SPLITTING.md §2): the
/// bundle's `manifest.json` is ONE object, each `tiles/<blake3>.zst` /
/// `blobs/<blake3>.zst` entry is its own parent-linked, content-addressed
/// object (Authoritative). This is the object that replaces
/// [`SCHEMA_DOC`] when `meta.document_encoding == Split`.
pub const SCHEMA_DOC_MANIFEST: &str = "photocraft.document.manifest.v1";
/// A split bundle's unsaved snapshot (over-cap dirty states only; journal-mode
/// unsaved edits stay one small [`SCHEMA_UNSAVED`] object).
pub const SCHEMA_UNSAVED_MANIFEST: &str = "photocraft.unsaved.manifest.v1";

pub const ADAPTER_ID: &str = "org.carryon.photocraft";

/// The PhotoCraft commit every carry must record for provenance
/// (`docs/artcraft-bridge/PHOTOCRAFT-PIN.md`).
pub const PHOTOCRAFT_PIN: &str = "1f7fc0f53696e54da79b95015abb37d7c22cdd1a";

/// Object-size ceiling for the manifest. The Carry-On core caps
/// `max_object_bytes` at 16 MiB today (registration rejects anything larger —
/// observed in the Phase-1 test run), so this matches the cap rather than
/// image-editor reality. A real `.pcraft` easily exceeds it; Phase 2 must
/// either raise the core cap or split documents into per-tile/per-layer
/// objects (ADAPTER-DESIGN §1 — noted, not claimed).
pub const MAX_OBJECT_BYTES: u64 = 16 * 1024 * 1024;

/// How the unsaved state is carried (ADAPTER-DESIGN §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnsavedMode {
    /// `.pcraft` snapshot of the dirty state (autosave-style; any driver).
    Snapshot,
    /// Command journal `[[id, params], …]` from the last saved revision
    /// (in-process driver only). UNSUPPORTED in this skeleton.
    Journal,
}

/// How the saved document is carried (TILE-SPLITTING.md §2). `Monolithic` is
/// the Phase 1–3 shape (one `.pcraft` zip object, any driver); `Split` is the
/// tile-split shape (manifest object + per-tile/per-blob objects).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DocumentEncoding {
    #[default]
    Monolithic,
    Split,
}

/// A `.pcraft` document bundle split at PhotoCraft's own seams (RESEARCH
/// §1.5): `manifest.json` plus the content-addressed entries it references.
///
/// `parts` is keyed by the bundle-relative entry path (`tiles/<blake3>.zst`,
/// `blobs/<blake3>.zst`); the bytes are the **exact entry bytes** PhotoCraft
/// wrote (zstd-compressed), never re-encoded. Previews (`thumb.png`,
/// `composite/preview.png`) are caller-rendered and deliberately absent — the
/// destination re-renders them (TILE-SPLITTING.md §2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentBundle {
    /// The bundle's `manifest.json` bytes (versioned document tree).
    pub manifest: Vec<u8>,
    /// Content-addressed entries, keyed by bundle path.
    pub parts: BTreeMap<String, Vec<u8>>,
}

/// The stable object id for a bundle part: `tiles/<blake3>.zst` →
/// `photocraft.tile.<blake3>.v1`, `blobs/<blake3>.zst` →
/// `photocraft.blob.<blake3>.v1`. The BLAKE3 name is PhotoCraft's own content
/// address for the entry (RESEARCH §1.5) — a stable logical id across
/// generations and machines. No filesystem path ever crosses this boundary
/// (digests, never executable paths).
pub fn part_object_id(bundle_path: &str) -> Option<String> {
    let kind = part_kind(bundle_path)?;
    let name = part_name(bundle_path)?;
    Some(format!("photocraft.{kind}.{name}.v1"))
}

fn part_kind(bundle_path: &str) -> Option<&'static str> {
    if bundle_path.starts_with("tiles/") {
        Some("tile")
    } else if bundle_path.starts_with("blobs/") {
        Some("blob")
    } else {
        None
    }
}

fn part_name(bundle_path: &str) -> Option<&str> {
    let file = bundle_path.rsplit('/').next()?;
    file.strip_suffix(".zst")
}

impl DocumentBundle {
    /// The hashes the manifest references (bare 64-hex BLAKE3 strings —
    /// PhotoCraft's manifest never embeds paths or binary payloads,
    /// RESEARCH §1.5/`manifest.rs`). A **diagnostic** only: the authoritative
    /// fail-closed closure check is PhotoCraft's own loader, which fetches
    /// every tile/blob it needs and refuses on the first missing one (the
    /// exact error shape the round-trip test pins).
    pub fn referenced_hashes(&self) -> Vec<String> {
        let mut refs = Vec::new();
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&self.manifest) {
            walk_strings(&v, &mut |s: &str| {
                if is_hash(s) {
                    refs.push(s.to_string());
                }
            });
        }
        refs.sort();
        refs.dedup();
        refs
    }

    /// The deterministic, machine-independent byte source for the
    /// authoritative-state hash (TILE-SPLITTING.md §2): len-prefixed
    /// `manifest.json`, then **every** part in sorted-path order (len-prefixed
    /// path, len-prefixed bytes). No zip metadata, no timestamps — byte-stable
    /// across machines, so the destination's re-derivation from imported state
    /// is exact. All parts (not a reference subset) because the manifest names
    /// parts by bare hash and PhotoCraft's loader — not this adapter — is the
    /// authority on which hashes it will fetch.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        extend_len_prefix(&mut out, &self.manifest);
        for (path, bytes) in &self.parts {
            extend_len_prefix(&mut out, path.as_bytes());
            extend_len_prefix(&mut out, bytes);
        }
        out
    }

    /// Every (object id, bundle path) the bundle declares as objects, sorted
    /// by object id (deterministic manifest order).
    pub fn part_entries(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .parts
            .keys()
            .filter_map(|p| part_object_id(p).map(|id| (id, p.clone())))
            .collect();
        out.sort();
        out
    }
}

/// A bare 64-hex string — the shape of PhotoCraft's content addresses
/// (`manifest.rs`: "Hex-encoded BLAKE3 hash (64 chars)").
fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn extend_len_prefix(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn walk_strings(v: &serde_json::Value, f: &mut dyn FnMut(&str)) {
    match v {
        serde_json::Value::String(s) => f(s),
        serde_json::Value::Array(a) => a.iter().for_each(|x| walk_strings(x, f)),
        serde_json::Value::Object(o) => o.values().for_each(|x| walk_strings(x, f)),
        _ => {}
    }
}

/// `photocraft.meta.v1` — portable identity + provenance.
///
/// Carries the engine-session facts PhotoCraft does not write into the file:
/// `revision` / `saved_revision` / `dirty` live in `DocState` only, and the
/// `.pcraft` manifest has no revision field (RESEARCH §1.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotocraftMeta {
    pub schema_id: String,
    pub schema_version: u32,
    pub adapter_version: String,
    /// The document's own path on the source machine, if any.
    pub original_path: Option<String>,
    pub document_name: String,
    /// `DocState.revision` — "Increments on every change" (RESEARCH §1.4).
    pub revision: u64,
    /// Revision at the last successful save (`DocState.saved_revision`).
    pub saved_revision: u64,
    /// `dirty == revision != saved_revision` (RESEARCH §1.4).
    pub dirty: bool,
    /// The PhotoCraft version/commit the bytes came from — the research pin
    /// (`docs/artcraft-bridge/PHOTOCRAFT-PIN.md`); an early-alpha project,
    /// so provenance matters.
    pub photocraft_pin: String,
    pub unsaved_mode: UnsavedMode,
    /// How the saved document is carried (TILE-SPLITTING.md §2). Defaults to
    /// the Phase 1–3 monolithic zip when absent (serde default), so carried
    /// state from earlier phases still reads.
    #[serde(default)]
    pub document_encoding: DocumentEncoding,
}

/// Portable layer target: name + zero-based occurrence, not a session-local
/// layer id — the form PhotoCraft actions record so recordings replay on other
/// documents (RESEARCH §1.3.2, control-protocol lines 627–635).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerTarget {
    pub name: String,
    pub occurrence: u32,
}

/// Viewport fragment of the workspace object: `ViewXform` is UI-crate state in
/// PhotoCraft (`crates/ui-egui/src/canvas/view_xform.rs`), readable/settable
/// only over the control channel (RESEARCH §1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// 1.0 = 100%, one document pixel per physical display pixel.
    pub zoom: f64,
    /// Normalized image center, `[u, v]` in `0..=1`.
    pub center: [f64; 2],
    /// View rotation in degrees.
    pub rotation: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            zoom: 1.0,
            center: [0.5, 0.5],
            rotation: 0.0,
        }
    }
}

/// `photocraft.workspace.v1` — the ephemeral working position: what makes the
/// destination open the same document at the same spot, never needed for
/// correctness (ADAPTER-DESIGN §1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    /// Active document/tab index; restore via
    /// `ui.menu.invoke {"id":"window.document.<index>"}`.
    pub active_document_index: u32,
    /// Selected/target layers, portable by name+occurrence.
    pub selected_layers: Vec<LayerTarget>,
    pub viewport: Viewport,
    /// Active tool id (e.g. "brush"); driver-scoped vocabulary.
    pub active_tool: String,
    /// Opaque panel/dock layout JSON as `ui.inspect` reports it — carried for
    /// UX restore only, never interpreted by the adapter.
    pub panels: serde_json::Value,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace {
            active_document_index: 0,
            selected_layers: Vec::new(),
            viewport: Viewport {
                zoom: 1.0,
                center: [0.5, 0.5],
                rotation: 0.0,
            },
            active_tool: "move".into(),
            panels: serde_json::json!({}),
        }
    }
}

/// Authoritative-state hash (ADAPTER-DESIGN §3): binds authority to state, not
/// to a bearer token or process identity. Length-prefixed concatenation, the
/// same discipline as `carryon-adapter-editor::EditorAdapter::auth_hash`.
pub fn authoritative_hash(document: &[u8], unsaved: &[u8], meta_bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update((document.len() as u64).to_le_bytes());
    h.update(document);
    h.update((unsaved.len() as u64).to_le_bytes());
    h.update(unsaved);
    h.update(meta_bytes);
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// SHA-256 of arbitrary object bytes (content_hash fields).
pub fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}
