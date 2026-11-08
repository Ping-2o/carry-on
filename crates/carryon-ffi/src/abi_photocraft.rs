//! PhotoCraft desktop-flagship UX entries (stage 10) — the live-session bridge
//! the macOS app drives: **inspect** a running PhotoCraft over its loopback
//! control channel (§18-style data-only dispatch), **capture** that session
//! into a Carry-On adapter (saved document + dirty-state snapshot + ephemeral
//! working position), and **restore** a carried session into a second live
//! PhotoCraft.
//!
//! Channel choice: PhotoCraft's **JSON control channel**, not its MCP server.
//! Both front the same engine commands, but the control channel is the surface
//! this adapter already pins and tests (whitelisted methods, loopback-only,
//! 0600 bearer-token file, non-destructive snapshot discipline) — MCP would add
//! a second protocol and a second security surface for the same verbs. See
//! `docs/artcraft-bridge/UX.md`.
//!
//! Integration is **fork-free**: nothing is linked from PhotoCraft at runtime
//! for this path. The app is driven through its own automation roots and the
//! adapter reads/writes only the user's selected document workspace.
//!
//! Secrets (EVD-006): the bearer token is read from its file inside Rust, is
//! never returned, logged, or placed in the transcript; the config JSON's
//! inline `token` exists for protocol tests only.
//!
//! Labels (§2 law): everything these entries produce is **OBSERVED (LOCAL)** —
//! loopback control channels and loopback TLS on one machine. No platform claim
//! rides along.

use crate::errors::{record, set_last_error, CARRYON_ERR_BAD_JSON};
use crate::ffi_guard;
use crate::handle::{as_mut, CarryonCore};
use crate::strings::{borrow_str, write_out_string};
use carryon_adapter_photocraft::control::{
    ControlChannelConfig, ControlChannelDriver, LiveRestoreReport,
};
use carryon_adapter_photocraft::live::reconstruct_carried;
use carryon_adapter_photocraft::model::Workspace;
use carryon_adapter_photocraft::PhotocraftAdapter;
use carryon_core::ids::CutId;
use carryon_core::model::ActionRequest;
use std::ffi::c_char;
use std::path::PathBuf;

const BAD_HANDLE: i32 = crate::errors::CARRYON_ERR_BAD_HANDLE;
const PANIC: i32 = crate::errors::CARRYON_ERR_PANIC;

/// One live/app config, as the shell passes it. `token` is a protocol-test
/// escape hatch; the app always uses `token_file` (EVD-006).
#[derive(serde::Deserialize)]
struct Config {
    addr: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    token_file: Option<String>,
    #[serde(default)]
    base_document: Option<String>,
    read_root: String,
    write_root: String,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    session_title: Option<String>,
    #[serde(default)]
    workspace_json: Option<String>,
    #[serde(default)]
    name_hint: Option<String>,
    #[serde(default)]
    screenshot: Option<String>,
}

impl Config {
    fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("bad photocraft config JSON: {e}"))
    }

    /// The control-channel config. The token is read here and lives only in a
    /// redacted `Secret` (EVD-006); the returned error never contains it.
    fn driver_config(&self) -> Result<ControlChannelConfig, String> {
        let token = match (&self.token, &self.token_file) {
            (_, Some(path)) => std::fs::read_to_string(path)
                .map_err(|e| format!("read control token file: {e}"))?
                .trim()
                .to_string(),
            (Some(t), None) => t.clone(),
            (None, None) => return Err("photocraft config needs token_file (or test token)".into()),
        };
        if token.is_empty() {
            return Err("control token is empty".into());
        }
        Ok(ControlChannelConfig {
            addr: self.addr.clone(),
            token: carryon_adapter_photocraft::Secret::new(token),
            base_document: self
                .base_document
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_default(),
            write_root: PathBuf::from(&self.write_root),
            read_root: PathBuf::from(&self.read_root),
        })
    }

    fn connect(&self) -> Result<ControlChannelDriver, String> {
        ControlChannelDriver::connect(self.driver_config()?).map_err(|e| e.to_string())
    }
}

/// Inspect a running PhotoCraft session (no capture, no writes). Writes a
/// token-free summary JSON into `*out_json`:
/// `{live:true, documents, active, name, path, revision, dirty, tool, zoom,
/// rotation, selected_layers}`.
///
/// # Safety
/// `cfg_json` valid UTF-8; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_photocraft_inspect(
    cfg_json: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let cfg = match borrow_str(cfg_json)
            .ok()
            .and_then(|s| Config::parse(s).ok())
        {
            Some(c) => c,
            None => {
                set_last_error("bad photocraft config JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let mut driver = match cfg.connect() {
            Ok(d) => d,
            Err(e) => {
                set_last_error(&e);
                return CARRYON_ERR_BAD_JSON;
            }
        };
        match driver.live_summary() {
            Ok(summary) => {
                let doc = serde_json::json!({
                    "live": true,
                    "addr": cfg.addr,
                    "documents": summary.documents,
                    "active": summary.active,
                    "name": summary.name,
                    "path": summary.path,
                    "revision": summary.revision,
                    "dirty": summary.dirty,
                    "tool": summary.tool,
                    "zoom": summary.zoom,
                    "rotation": summary.rotation,
                    "selected_layers": summary.selected_layers,
                });
                write_out_string(out_json, doc.to_string())
            }
            Err(e) => {
                set_last_error(e.to_string());
                CARRYON_ERR_BAD_JSON
            }
        }
    })
}

/// Capture the live session and register the PhotoCraft adapter on `core`
/// (source side). Writes `{adapter_id, encoding, name, revision, dirty,
/// parts, document_bytes, unsaved_bytes, total_bytes, workspace_json,
/// capture:{…}}`.
///
/// `encoding`: `"split"` (the saved `.pcraft` ZIP is split into PhotoCraft's
/// own content-addressed tiles) or `"monolithic"` (the ZIP as one object).
///
/// # Safety
/// `core` live; `cfg_json` valid UTF-8; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_photocraft_register(
    core: *mut CarryonCore,
    cfg_json: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let cfg = match borrow_str(cfg_json)
            .ok()
            .and_then(|s| Config::parse(s).ok())
        {
            Some(c) => c,
            None => {
                set_last_error("bad photocraft config JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let split = match cfg.encoding.as_deref() {
            None | Some("monolithic") | Some("zip") => false,
            Some("split") | Some("tiles") => true,
            Some(other) => {
                set_last_error(format!(
                    "photocraft encoding {other:?} (use \"split\" or \"monolithic\")"
                ));
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let mut driver = match cfg.connect() {
            Ok(d) => d,
            Err(e) => {
                set_last_error(&e);
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let capture = match driver.capture(split) {
            Ok(c) => c,
            Err(e) => {
                set_last_error(e.to_string());
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let title = cfg
            .session_title
            .clone()
            .unwrap_or_else(|| format!("PhotoCraft: {}", capture.report.name));
        let adapter: PhotocraftAdapter = match capture.bundle.clone() {
            Some(bundle) => PhotocraftAdapter::with_split_state(
                title,
                bundle,
                capture.unsaved.clone(),
                capture.workspace.clone(),
                capture.meta.clone(),
            ),
            None => PhotocraftAdapter::with_state(
                title,
                capture.document.clone(),
                capture.unsaved.clone(),
                capture.workspace.clone(),
                capture.meta.clone(),
            ),
        };
        let info = match core.register_adapter(Box::new(adapter)) {
            Ok(i) => i,
            Err(e) => return record(&e),
        };
        let report = &capture.report;
        let out = serde_json::json!({
            "adapter_id": info.adapter_id,
            "encoding": report.encoding,
            "name": report.name,
            "revision": report.revision,
            "dirty": report.dirty,
            "path": report.path,
            "parts": report.parts,
            "document_bytes": report.document_bytes,
            "unsaved_bytes": report.unsaved_bytes,
            "total_bytes": report.total_bytes,
            "tool": report.tool,
            "zoom": report.zoom,
            "rotation": report.rotation,
            "selected_layers": report.selected_layers,
            "workspace_json": serde_json::to_string(&capture.workspace).unwrap_or_else(|_| "{}".into()),
            "capture": report,
        });
        write_out_string(out_json, out.to_string())
    })
}

/// Restore an imported PhotoCraft session into a second LIVE PhotoCraft.
/// Reconstructs the carried state from THIS core's store by the sealed cut's
/// object digests, opens it in the app behind `cfg`, reapplies the carried
/// working position, and runs the adapter's `session.restore` so the engine's
/// oracle re-derives the authoritative-state hash from the imported state.
/// Writes `{restored:{…}, live_restore:true, oracle:{…}, objects, bytes,
/// workspace:"carried"|"default"}`.
///
/// `mirror_session` is the local mirror session id (not the remote one);
/// `mirror_cut` is the local mirror cut number.
///
/// # Safety
/// `core` live; `cfg_json` valid UTF-8; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_photocraft_restore(
    core: *mut CarryonCore,
    cfg_json: *const c_char,
    mirror_session: *const c_char,
    mirror_cut: u64,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let cfg = match borrow_str(cfg_json)
            .ok()
            .and_then(|s| Config::parse(s).ok())
        {
            Some(c) => c,
            None => {
                set_last_error("bad photocraft config JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let mirror = match borrow_str(mirror_session) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let session = match uuid::Uuid::parse_str(mirror) {
            Ok(u) => carryon_core::ids::SessionId(u),
            Err(_) => {
                set_last_error("bad mirror session id");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let cut = CutId {
            session,
            number: mirror_cut,
        };
        let rows = match core.cut_object_digests(cut) {
            Ok(r) => r,
            Err(e) => return record(&e),
        };
        let carried = match reconstruct_carried(&rows, |hex| {
            core.read_object_hex(hex).map_err(|e| e.to_string())
        }) {
            Ok(c) => c,
            Err(e) => {
                set_last_error(&e);
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let workspace = match cfg.workspace_json.as_deref() {
            Some(json) => serde_json::from_str::<Workspace>(json).unwrap_or_default(),
            None => Workspace::default(),
        };
        let workspace_carried = cfg.workspace_json.is_some();
        let name_hint = cfg
            .name_hint
            .clone()
            .unwrap_or_else(|| carried.meta.document_name.clone());

        // The destination app is driven through its own control channel.
        let mut driver = match cfg.connect() {
            Ok(d) => d,
            Err(e) => {
                set_last_error(&e);
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let restored: LiveRestoreReport = match driver.restore_into_live(
            carried.unsaved.as_deref(),
            carried.bundle.as_ref(),
            &workspace,
            &name_hint,
            cfg.screenshot.as_deref(),
        ) {
            Ok(r) => r,
            Err(e) => {
                set_last_error(e.to_string());
                return CARRYON_ERR_BAD_JSON;
            }
        };

        // Register the reconstructed adapter (with the same live driver) and run
        // the destination's `session.restore`: the action's oracle re-derives
        // the authoritative-state hash from the imported state and must agree
        // with the sealed one — a restore that only opened a file does not pass.
        let adapter = match carried.bundle.clone() {
            Some(bundle) => PhotocraftAdapter::with_split_state(
                name_hint.clone(),
                bundle,
                carried.unsaved.clone().unwrap_or_default(),
                workspace.clone(),
                carried.meta.clone(),
            ),
            None => PhotocraftAdapter::with_state(
                name_hint.clone(),
                carried.document.clone().unwrap_or_default(),
                carried.unsaved.clone().unwrap_or_default(),
                workspace.clone(),
                carried.meta.clone(),
            ),
        }
        .with_driver(Box::new(driver));
        let info = match core.register_adapter(Box::new(adapter)) {
            Ok(i) => i,
            Err(e) => return record(&e),
        };
        let action = match core.execute_action_as(
            cut,
            ActionRequest {
                class: "session.restore".into(),
                params: serde_json::json!({}),
            },
            &info.adapter_id,
        ) {
            Ok(a) => a,
            Err(e) => return record(&e),
        };
        let out = serde_json::json!({
            "restored": restored,
            "live_restore": action.output.get("live_restore").cloned().unwrap_or(serde_json::Value::Bool(true)),
            "oracle": {
                "checked": action.oracle.checked,
                "agreed": action.oracle.agreed,
                "detail": action.oracle.detail,
            },
            "workspace": if workspace_carried { "carried" } else { "default" },
            "objects": carried.objects,
            "bytes": carried.bytes,
            "adapter_id": info.adapter_id,
        });
        write_out_string(out_json, out.to_string())
    })
}
