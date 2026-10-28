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

/// Saved document as single-file `.pcraft` bytes (Authoritative).
pub const SCHEMA_DOC: &str = "photocraft.document.v1";
/// Unsaved edits (Authoritative, parents = document).
pub const SCHEMA_UNSAVED: &str = "photocraft.unsaved_edits.v1";
/// Identity/provenance meta (Authoritative).
pub const SCHEMA_META: &str = "photocraft.meta.v1";
/// Ephemeral working position (Ephemeral/optional).
pub const SCHEMA_WORKSPACE: &str = "photocraft.workspace.v1";

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
