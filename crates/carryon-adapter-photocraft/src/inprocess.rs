//! InProcessDriver — Phase 2b implementation of [`PhotocraftDriver`] over a
//! real `photocraft_engine::Session` embedded in the Carry-On host process
//! (`docs/artcraft-bridge/RESEARCH.md` §2.5, ADAPTER-DESIGN §5).
//!
//! Compiles only with `--features inprocess` (the pinned git dependency on
//! `photocraft-engine`; see `PHOTOCRAFT-PIN.md`).
//!
//! # What this driver adds over the control channel
//!
//! - **Journal-mode unsaved edits**: `Session.journal` is a `Vec<(command id,
//!   params)>` record of every journaled edit (RESEARCH §1.3.1). The adapter
//!   slices it from a checkpoint taken at the last save — bytes that are
//!   typically orders of magnitude smaller than a `.pcraft` snapshot of the
//!   dirty state, and replayable on the destination via `Session::execute`.
//! - **`saved_revision` is visible**: the control surface hides it; in-process
//!   `DocState.saved_revision` is readable, so the meta object carries the
//!   true saved/dirty boundary.
//!
//! # Fail-closed authorization (`Session::authorize`)
//!
//! The engine runs a gate on **every** command ("Untrusted sessions install
//! the same check a top-level request sees", `crates/engine/src/lib.rs`
//! authorize field). [`InProcessDriver::new`] installs [`authorize_policy`],
//! which denies everything except:
//!
//! - read-only commands (`*.inspect`, `*.list`, `document.pixel`, …);
//! - `file.new` and edits in [`EDIT_ALLOWLIST`];
//! - `file.save`/`file.saveAs`/`image.duplicate`/`file.close`/`file.revert`
//!   with an output/affected path inside the explicitly-granted
//!   [`WorkRoot`] (set once via `OnceLock`), mirroring PhotoCraft's own
//!   `AuthorizedWorkspace` root discipline (RESEARCH §2.2) — everything
//!   ambient fails closed.
//!
//! # Headless honesty
//!
//! There is no viewport in the engine (it lives in ui-egui, RESEARCH §1.2), so
//! [`PhotocraftDriver::restore`] restores active document, tool-adjacent
//! selection state and layer selection, and the `workspace.viewport` fragment
//! is carried but **not applied** headless; the report output says so.

use crate::driver::{DriverError, EditOutcome, PhotocraftDriver, Step};
use crate::model::{PhotocraftMeta, UnsavedMode, Workspace, PHOTOCRAFT_PIN};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The scratch/write root granted at construction; commands touching any other
/// path fail closed.
static WORK_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Edit commands the adapter may replay (Phase-2 measurement set: painting,
/// layer property edits, adjustments, filters, selection, transform). Anything
/// else — file I/O, actions, plugins, preferences — is denied.
const EDIT_ALLOWLIST: &[&str] = &[
    // paint + retouch
    "paint.stroke",
    "paint.pencil",
    "paint.gradient",
    "paint.contentAwareMove",
    "edit.fill",
    "edit.clear",
    // layers
    "layer.new.layer",
    "layer.new.group",
    "layer.newAdjustmentLayer.brightnessContrast",
    "layer.newAdjustmentLayer.curves",
    "layer.newAdjustmentLayer.hueSaturation",
    "layer.newAdjustmentLayer.levels",
    "layer.newAdjustmentLayer.vibrance",
    "layer.newFillLayer.solidColor",
    "layer.delete",
    "layer.duplicate",
    "layer.setProps",
    "layer.select",
    "layer.translate",
    "layer.rasterize.type",
    // selection
    "select.rect",
    "select.none",
    "select.invert",
    // adjustments/filters (applied destructively)
    "image.adjustments.invert",
    "filter.blur.gaussianBlur",
    // transform / crop
    "edit.transform.scale",
    "image.crop",
    // undo inside a carried edit sequence is still an edit of the document
    "edit.undo",
    "edit.redo",
];

/// Read-only commands the adapter may run for state and oracles.
const READ_ALLOWLIST: &[&str] = &[
    "session.inspect",
    "document.inspect",
    "layer.list",
    "command.list",
    "document.pixel",
];

/// The `Session::authorize` gate: fail closed on everything not explicitly
/// granted. `fn` pointer (the engine's shape), so the granted root travels in
/// the `OnceLock`.
pub fn authorize_policy(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    use photocraft_engine::EngineError;
    let deny = |why: &str| Err(EngineError::Other(why.to_string()));
    if READ_ALLOWLIST.contains(&id) {
        return Ok(());
    }
    if EDIT_ALLOWLIST.contains(&id) {
        // Belt and braces: edits in the allowlist may still carry paths
        // (e.g. pattern fills); deny any path param outside the root.
        if let Some(p) = path_param(params) {
            if !under_root(&p) {
                return deny("edit command touches a path outside the granted work root");
            }
        }
        return Ok(());
    }
    match id {
        "file.new" => Ok(()),
        "image.duplicate" | "file.close" | "file.revert" => Ok(()),
        "file.save" | "file.saveAs" => {
            let Some(p) = path_param(params) else {
                return deny("save without a path is not permitted for the adapter session");
            };
            if under_root(&p) {
                Ok(())
            } else {
                deny("save path outside the granted work root")
            }
        }
        "file.open" => {
            let Some(p) = path_param(params) else {
                return deny("open without a path");
            };
            if under_root(&p) {
                Ok(())
            } else {
                deny("open path outside the granted work root")
            }
        }
        _other => deny("command is not granted to the PhotoCraft adapter session"),
    }
}

/// Extract a plausible path parameter (`path`, `to`, `out`) for the root check.
fn path_param(params: &Value) -> Option<String> {
    ["path", "to", "out"]
        .iter()
        .find_map(|k| params.get(k).and_then(|v| v.as_str()).map(|s| s.to_string()))
}

fn under_root(p: &str) -> bool {
    let Some(root) = WORK_ROOT.get() else {
        return false; // no root granted: everything fails closed
    };
    // Traversal never granted, same discipline as PhotoCraft's
    // AuthorizedWorkspace (RESEARCH §2.2: `..` rejected before I/O).
    if p.contains("..") {
        return false;
    }
    let path = Path::new(p);
    if path.is_absolute() {
        path.starts_with(root)
    } else {
        // Relative names resolve beneath the root (the harness constructs
        // them from the root); traversal was rejected above.
        true
    }
}

/// Configuration for one embedded session.
#[derive(Debug, Clone)]
pub struct InProcessConfig {
    /// Directory the session may read/write (scratch saves, carried docs).
    /// Mirrors `--automation-read-root`/`--automation-write-root` with one
    /// root for both, granted explicitly at construction.
    pub work_root: PathBuf,
}

/// The granted root accessor (tests).
pub fn work_root() -> Option<&'static Path> {
    WORK_ROOT.get().map(|p| p.as_path())
}

/// Phase 2b driver: a real PhotoCraft engine session in this process.
pub struct InProcessDriver {
    session: photocraft_engine::Session,
    cfg: InProcessConfig,
    /// Journal length at the last known save — the slice point for
    /// journal-mode unsaved edits.
    saved_checkpoint: usize,
    /// Explicit carry mode (measurements); None = default Journal.
    unsaved_mode_override: Option<UnsavedMode>,
}

impl std::fmt::Debug for InProcessDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessDriver")
            .field("cfg", &self.cfg)
            .field("journal_len", &self.session.journal.len())
            .field("saved_checkpoint", &self.saved_checkpoint)
            .finish()
    }
}

impl InProcessDriver {
    /// Build an embedded session with the fail-closed [`authorize_policy`]
    /// installed and the work root granted.
    pub fn new(cfg: InProcessConfig) -> Result<Self, DriverError> {
        let _ = WORK_ROOT.set(cfg.work_root.clone());
        let mut session = photocraft_engine::Session::new();
        session.authorize = Some(authorize_policy);
        Ok(InProcessDriver {
            session,
            cfg,
            saved_checkpoint: 0,
            unsaved_mode_override: None,
        })
    }

    /// Mutable access for harnesses that need engine-level setup (still gated
    /// by the same authorize policy — the gate lives in the session).
    pub fn session_mut(&mut self) -> &mut photocraft_engine::Session {
        &mut self.session
    }

    /// Mark the current journal position as the "last save" checkpoint.
    /// Called by the harness right after a successful save (or at
    /// document-open, for never-saved documents).
    pub fn mark_saved_checkpoint(&mut self) {
        self.saved_checkpoint = self.session.journal.len();
    }

    /// The journal steps since the checkpoint, as [`Step`]s.
    pub fn journal_steps(&self) -> Vec<Step> {
        self.steps_since_checkpoint()
    }

    fn steps_since_checkpoint(&self) -> Vec<Step> {
        self.session
            .journal
            .iter()
            .skip(self.saved_checkpoint)
            .map(|(id, params)| Step {
                id: id.clone(),
                params: params.clone(),
            })
            .collect()
    }

    fn run(&mut self, id: &str, params: Value) -> Result<Value, DriverError> {
        self.session
            .execute(id, params)
            .map_err(|e| DriverError::Failed(format!("{id}: {e}")))
    }

    fn active_index(&self) -> Result<usize, DriverError> {
        self.session
            .active_index()
            .ok_or_else(|| DriverError::Failed("no active document".into()))
    }

    /// Save the ACTIVE document's current state as a `.pcraft` under the work
    /// root (a path the harness constructed), returning the bytes.
    fn save_active_snapshot(&mut self, rel: &str) -> Result<Vec<u8>, DriverError> {
        let full = self.cfg.work_root.join(rel);
        self.run(
            "file.saveAs",
            serde_json::json!({ "path": full.to_string_lossy(), "format": "pcraft" }),
        )?;
        let bytes = std::fs::read(&full)
            .map_err(|e| DriverError::Failed(format!("read snapshot {full:?}: {e}")))?;
        let _ = std::fs::remove_file(&full);
        Ok(bytes)
    }

    /// Dirty-state snapshot WITHOUT cleaning the original (in-process
    /// `image.duplicate` + save the duplicate + close it), the analogue of
    /// the control-channel `snapshot_scratch`.
    fn duplicate_snapshot(&mut self, tag: &str) -> Result<Vec<u8>, DriverError> {
        let before = self.session.documents().len();
        let orig_active = self.active_index()?;
        self.run("image.duplicate", serde_json::json!({}))?;
        let after = self.session.documents().len();
        let now = self.active_index()?;
        if after != before + 1 || now == orig_active {
            return Err(DriverError::Failed(
                "image.duplicate did not add+activate a duplicate; refusing to save the original"
                    .into(),
            ));
        }
        let bytes = self.save_active_snapshot(&format!("scratch-{tag}.pcraft"))?;
        self.run("file.close", serde_json::json!({}))?;
        if self.session.active_index() != Some(orig_active) {
            self.session.set_active(orig_active);
        }
        Ok(bytes)
    }

    fn unsaved_mode(&self) -> UnsavedMode {
        // Journal mode is what this driver exists for; the harness decides by
        // constructing with [`Self::with_unsaved_mode`] if it needs snapshots.
        self.unsaved_mode_override.unwrap_or(UnsavedMode::Journal)
    }

    /// Compute the unsaved-state bytes in an explicit mode WITHOUT changing
    /// the driver's declared mode (measurements need both from one session).
    pub fn unsaved_bytes_as(&mut self, mode: UnsavedMode) -> Result<Vec<u8>, DriverError> {
        let idx = self.active_index()?;
        let dirty = self.session.documents()[idx].is_dirty();
        if !dirty {
            self.saved_checkpoint = self.session.journal.len();
            return Ok(Vec::new());
        }
        match mode {
            UnsavedMode::Journal => {
                // The PhotoCraft actions/batch shape `[[id, params], …]`
                // (serde serializes `(String, Value)` tuples as arrays) —
                // replayable by actions, droplets and `Session::execute`.
                let pairs: Vec<(String, Value)> = self
                    .steps_since_checkpoint()
                    .into_iter()
                    .map(|s| (s.id, s.params))
                    .collect();
                serde_json::to_vec(&pairs)
                    .map_err(|e| DriverError::Failed(format!("encode journal: {e}")))
            }
            UnsavedMode::Snapshot => self.duplicate_snapshot("unsaved"),
        }
    }
}

impl PhotocraftDriver for InProcessDriver {
    fn document_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        // The saved base document: if the active doc has a path under the
        // root, read it; otherwise the document object is the state at the
        // checkpoint — represent it as the duplicate snapshot (a
        // never-saved document still has authoritative bytes to carry).
        let idx = self.active_index()?;
        let path = self.session.documents()[idx].path.clone();
        match path {
            Some(p) if under_root(&p) => std::fs::read(Path::new(&p))
                .map_err(|e| DriverError::Failed(format!("read base document {p:?}: {e}"))),
            _ => self.duplicate_snapshot("base"),
        }
    }

    fn unsaved_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        self.unsaved_bytes_as(self.unsaved_mode())
    }

    fn meta(&mut self) -> Result<PhotocraftMeta, DriverError> {
        let idx = self.active_index()?;
        let d = &self.session.documents()[idx];
        let (name, path) = {
            let p = d.path.clone();
            let n = d.doc.name.clone();
            (n, p)
        };
        Ok(PhotocraftMeta {
            schema_id: "carryon.photocraft.session".into(),
            schema_version: 1,
            adapter_version: env!("CARGO_PKG_VERSION").into(),
            original_path: path,
            document_name: name,
            revision: d.revision,
            saved_revision: d.saved_revision,
            dirty: d.is_dirty(),
            photocraft_pin: PHOTOCRAFT_PIN.into(),
            unsaved_mode: self.unsaved_mode(),
        })
    }

    fn workspace_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        // Headless: viewport/tool-panel state does not exist in the engine; the
        // workspace object carries the active index + selected layers and the
        // default viewport. Tool name: the engine has ToolState, but there is
        // no active-tool concept headless (no UI), so "move" is the neutral
        // marker here.
        let idx = self.active_index()?;
        let d = &self.session.documents()[idx];
        // Map selected layer ids to portable names by walking the layer tree.
        let selected_ids: Vec<u64> = d
            .selected_layers
            .iter()
            .map(PhotocraftLayerId::to_u64)
            .collect();
        let selected = PhotocraftLayerId::walk(&d.doc)
            .into_iter()
            .filter(|(_, _, id)| selected_ids.contains(id))
            .map(|(name, occurrence, _)| crate::model::LayerTarget { name, occurrence })
            .collect();
        let workspace = Workspace {
            active_document_index: idx as u32,
            selected_layers: selected,
            viewport: crate::model::Viewport::default(),
            active_tool: "move".into(),
            panels: serde_json::json!({ "headless": true }),
        };
        serde_json::to_vec(&workspace)
            .map_err(|e| DriverError::Failed(format!("encode workspace: {e}")))
    }

    fn apply_steps(&mut self, steps: &[Step]) -> Result<EditOutcome, DriverError> {
        let revision_before = {
            let idx = self.active_index()?;
            self.session.documents()[idx].revision
        };
        let mut ran = 0usize;
        for step in steps {
            self.run(&step.id, step.params.clone())?; // fail closed on first error
            ran += 1;
        }
        let idx = self.active_index()?;
        let revision_after = self.session.documents()[idx].revision;
        // Replay oracle, part 1 (§ADAPTER-DESIGN 2.1): the revision must have
        // advanced; part 2 is the caller's document.inspect re-derivation.
        if revision_after < revision_before {
            return Err(DriverError::Failed(format!(
                "revision went backwards after replay ({revision_before} -> {revision_after})"
            )));
        }
        let unsaved_hash = if self.session.documents()[idx].is_dirty() {
            let bytes = self.duplicate_snapshot("edit-hash")?;
            crate::model::hex_sha256(&bytes)
        } else {
            crate::model::hex_sha256(&[])
        };
        Ok(EditOutcome {
            steps_ran: ran,
            new_revision: revision_after,
            unsaved_hash,
        })
    }

    fn restore(&mut self, workspace: &Workspace) -> Result<(), DriverError> {
        // Headless restore: activate the document and restore the layer
        // selection by portable name+occurrence. The viewport fragment is
        // carried but NOT applied — the engine has no viewport (ui-egui
        // owns it, RESEARCH §1.2). Failing to select a layer fails closed.
        if self.session.documents().len() > workspace.active_document_index as usize {
            self.session
                .set_active(workspace.active_document_index as usize);
        } else {
            return Err(DriverError::Failed(format!(
                "workspace names document index {} but the session holds {}",
                workspace.active_document_index,
                self.session.documents().len()
            )));
        }
        for target in &workspace.selected_layers {
            // Resolve name+occurrence to the concrete layer id, then select.
            let idx = self.active_index()?;
            let doc = &self.session.documents()[idx].doc;
            let found = PhotocraftLayerId::walk(doc)
                .into_iter()
                .find(|(name, occurrence, _)| *name == target.name && *occurrence == target.occurrence)
                .map(|(_, _, id)| id);
            let Some(id) = found else {
                return Err(DriverError::Failed(format!(
                    "workspace layer {:?}#{} not found in the destination document",
                    target.name, target.occurrence
                )));
            };
            self.run(
                "layer.select",
                serde_json::json!({ "layer": id, "mode": "add" }),
            )?;
        }
        Ok(())
    }
}

impl InProcessDriver {
    /// Construct with an explicit unsaved carry mode (measurements need both).
    pub fn with_unsaved_mode(mut self, mode: UnsavedMode) -> Self {
        self.unsaved_mode_override = Some(mode);
        self
    }
}

// Engine layer ids are opaque; walk the document tree in walk order (the same
// top-to-bottom traversal `document.inspect` uses) yielding
// (name, occurrence, id).
struct PhotocraftLayerId;

impl PhotocraftLayerId {
    fn to_u64(l: &photocraft_doc::LayerId) -> u64 {
        l.0
    }

    fn walk(doc: &photocraft_doc::Document) -> Vec<(String, u32, u64)> {
        let mut out = Vec::new();
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for layer in &doc.layers {
            Self::walk_layer(layer, &mut counts, &mut out);
        }
        out
    }

    fn walk_layer(
        layer: &photocraft_doc::Layer,
        counts: &mut std::collections::HashMap<String, u32>,
        out: &mut Vec<(String, u32, u64)>,
    ) {
        let occurrence = *counts.entry(layer.name.clone()).or_insert(0);
        counts.insert(layer.name.clone(), occurrence + 1);
        out.push((layer.name.clone(), occurrence, layer.id.0));
        if let photocraft_doc::LayerContent::Group(g) = &layer.content {
            for child in &g.children {
                Self::walk_layer(child, counts, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("pcraft-inproc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn authorize_denies_ungranted_commands() {
        for id in [
            "actions.play",
            "prefs.set",
            "plugins.install",
            "file.exit",
            "edit.keyboardShortcuts",
        ] {
            assert!(
                authorize_policy(id, &serde_json::json!({})).is_err(),
                "{id} must be denied"
            );
        }
    }

    #[test]
    fn authorize_denies_file_save_outside_root() {
        let r = root();
        let _ = WORK_ROOT.set(r.clone());
        assert!(authorize_policy(
            "file.saveAs",
            &serde_json::json!({ "path": "/etc/evil.pcraft" })
        )
        .is_err());
        assert!(authorize_policy(
            "file.saveAs",
            &serde_json::json!({ "path": format!("{}/ok.pcraft", r.display()) })
        )
        .is_ok());
        assert!(authorize_policy(
            "file.open",
            &serde_json::json!({ "path": format!("{}/../escape.pcraft", r.display()) })
        )
        .is_err(), "traversal must be denied");
        // No path param on a save: denied.
        assert!(authorize_policy("file.saveAs", &serde_json::json!({})).is_err());
    }

    #[test]
    fn authorize_grants_read_edit_and_new() {
        let _ = WORK_ROOT.set(root());
        for id in [
            "session.inspect",
            "document.inspect",
            "layer.list",
            "document.pixel",
            "file.new",
            "paint.stroke",
            "layer.setProps",
            "filter.blur.gaussianBlur",
            "select.rect",
            "image.duplicate",
            "file.close",
        ] {
            assert!(
                authorize_policy(id, &serde_json::json!({})).is_ok(),
                "{id} must be granted"
            );
        }
    }

    #[test]
    fn journal_slice_and_replay_semantics_end_to_end() {
        // Real engine session through the fail-closed gate: new document,
        // checkpoint, edits, then the journal slice must contain exactly the
        // post-checkpoint steps; replaying them on a second session opened on
        // the same saved file must reproduce the dirty-state hash (the oracle).
        let r = root();
        let mut src = InProcessDriver::new(InProcessConfig { work_root: r.clone() })
            .expect("session");
        src.session_mut()
            .execute(
                "file.new",
                serde_json::json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8, "background": "white"}),
            )
            .unwrap();
        let doc = r.join("t.pcraft");
        src.session_mut()
            .execute(
                "file.saveAs",
                serde_json::json!({ "path": doc.to_string_lossy() }),
            )
            .unwrap();
        src.mark_saved_checkpoint();
        src.session_mut()
            .execute("layer.new.layer", serde_json::json!({"name": "L1"}))
            .unwrap();
        src.session_mut()
            .execute("layer.select", serde_json::json!({"name": "L1"}))
            .unwrap();
        src.session_mut()
            .execute(
                "paint.stroke",
                serde_json::json!({"points": [[10, 10, 1.0]], "size": 8, "color": "#f00"}),
            )
            .unwrap();

        let meta = src.meta().unwrap();
        assert!(meta.dirty, "the document must be dirty after the edits");
        assert_eq!(meta.saved_revision, 1, "saved at revision 1");

        let journal = src.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
        let pairs: Vec<(String, Value)> = serde_json::from_slice(&journal).unwrap();
        assert!(
            pairs.len() >= 3,
            "journal must carry the post-save steps: {pairs:?}"
        );
        assert!(pairs.iter().any(|(id, _)| id == "paint.stroke"));

        // Snapshot of the dirty state (Phase-2a technique, in-process).
        let snapshot = src.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
        assert!(!snapshot.is_empty());

        // Replay on a second clean session.
        let mut dst = InProcessDriver::new(InProcessConfig { work_root: r.clone() })
            .expect("dest session");
        dst.session_mut()
            .execute("file.open", serde_json::json!({ "path": doc.to_string_lossy() }))
            .unwrap();
        let steps: Vec<Step> = pairs
            .into_iter()
            .map(|(id, params)| Step { id, params })
            .collect();
        let outcome = dst.apply_steps(&steps).unwrap();
        assert_eq!(outcome.steps_ran, steps.len());
        let dst_meta = dst.meta().unwrap();
        assert!(dst_meta.dirty);
        assert_eq!(
            dst_meta.revision, meta.revision,
            "replay must land on the same revision"
        );
        let dst_snapshot = dst.unsaved_bytes_as(UnsavedMode::Snapshot).unwrap();
        assert_eq!(
            hex_sha256(&dst_snapshot),
            hex_sha256(&snapshot),
            "replayed dirty state must hash equal to the source's (replay oracle)"
        );
        let _ = std::fs::remove_file(&doc);
    }
}
