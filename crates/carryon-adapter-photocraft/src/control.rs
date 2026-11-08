//! ControlChannelDriver — Phase 2a implementation of [`PhotocraftDriver`] over
//! the PhotoCraft desktop app's JSON-lines loopback control channel
//! (`docs/artcraft-bridge/RESEARCH.md` §2.3).
//!
//! # Security posture
//!
//! - **EVD-006 (token secrecy)**: the bearer token lives in [`Secret`], whose
//!   `Debug`/`Display` are redacted. It never appears in errors, transcripts,
//!   or evidence; the auth frame is excluded from the transcript.
//! - **Method whitelist**: only the methods in [`ALLOWED_METHODS`] can be
//!   called; anything else fails closed before a byte is written. The
//!   *command* layer behind `engine.execute` is additionally gated by
//!   PhotoCraft's own fail-closed automation authorize hook.
//! - Loopback only: the caller passes `127.0.0.1:<port>`; we refuse non-
//!   loopback addresses (`docs/control-protocol.md` security note).
//!
//! # Live mapping (RESEARCH §1/§2)
//!
//! - document bytes: the last-saved file, read directly from the adapter-side
//!   base path (same machine, user-selected — same consent class as
//!   `user_selected_file`); the control protocol has no "fetch file bytes"
//!   method, and `app.save` writes the *current* (dirty) state.
//! - unsaved bytes: if dirty, `app.save` writes a `.pcraft` snapshot of the
//!   dirty state under the app's `--automation-write-root`; the driver reads
//!   it back and removes the scratch file. If clean: empty bytes.
//! - meta: `session.inspect` (via `engine.execute`) gives per-document
//!   `name/path/dirty/revision`; `saved_revision` is not exposed by the live
//!   surface and is carried as `0` (documented in `model::PhotocraftMeta`).
//! - workspace: one `ui.inspect` call yields tool, views (zoom/center/
//!   rotation, per document index), the session list and the active
//!   document's layer tree (names + selected) — everything
//!   [`crate::model::Workspace`] needs.

use crate::driver::{DriverError, EditOutcome, PhotocraftDriver, Step};
use crate::model::{DocumentBundle, PhotocraftMeta, UnsavedMode, Viewport, Workspace};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Methods this driver may ever call. Fail closed on anything else.
const ALLOWED_METHODS: &[&str] = &[
    "app.open",
    "app.save",
    "app.quit",
    "engine.execute",
    "engine.commands",
    "ui.inspect",
    "ui.set",
    "ui.menu.invoke",
    "ui.screenshot",
    "ui.focus",
];

const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// The server rejects reply lines over 8 MiB (`docs/control-protocol.md`).
const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;

/// A bearer token that can never leak through Debug/Display/evidence (EVD-006).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(<redacted, {} chars>)", self.0.len())
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<redacted>")
    }
}

/// Connection to one running PhotoCraft desktop app.
struct ControlClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl ControlClient {
    /// Connect and authenticate (first frame on every connection).
    fn connect(addr: &str, token: &Secret) -> Result<Self, DriverError> {
        let stream = TcpStream::connect(addr)
            .map_err(|e| DriverError::Failed(format!("connect {addr}: {e}")))?;
        let peer = stream
            .peer_addr()
            .map_err(|e| DriverError::Failed(format!("peer addr: {e}")))?;
        if !peer.ip().is_loopback() {
            return Err(DriverError::Failed(format!(
                "refusing non-loopback control address {peer}"
            )));
        }
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|_| stream.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|e| DriverError::Failed(format!("set timeouts: {e}")))?;
        let writer = stream
            .try_clone()
            .map_err(|e| DriverError::Failed(format!("clone stream: {e}")))?;
        let mut c = ControlClient {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
        };
        // Auth: `{"id":"auth","method":"auth","params":{"token":...}}`. The
        // token goes on the wire and nowhere else.
        let frame = c.round_trip("auth", &serde_json::json!({ "token": token.as_str() }))?;
        if !frame.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(DriverError::Failed("control auth refused".into()));
        }
        Ok(c)
    }

    /// One request/reply round trip. Whitelisted methods only.
    fn call(
        &mut self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, DriverError> {
        if method != "auth" && !ALLOWED_METHODS.contains(&method) {
            return Err(DriverError::Failed(format!(
                "method {method:?} is not whitelisted for the PhotoCraft control driver"
            )));
        }
        let frame = self.round_trip(method, params)?;
        Ok(frame
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// Full reply frame (`{"id","ok","result"|"error"}`), errors mapped.
    fn round_trip(
        &mut self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, DriverError> {
        let id = format!("call-{}", self.next_id);
        self.next_id += 1;
        let frame = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&frame)
            .map_err(|e| DriverError::Failed(format!("encode {method}: {e}")))?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| DriverError::Failed(format!("send {method}: {e}")))?;
        let mut buf = String::new();
        let n = self
            .reader
            .read_line(&mut buf)
            .map_err(|e| DriverError::Failed(format!("recv {method}: {e}")))?;
        if n == 0 {
            return Err(DriverError::Failed(format!(
                "control connection closed during {method}"
            )));
        }
        if buf.len() > MAX_REPLY_BYTES {
            return Err(DriverError::Failed(format!(
                "reply over {MAX_REPLY_BYTES} bytes for {method}"
            )));
        }
        let reply: serde_json::Value = serde_json::from_str(&buf)
            .map_err(|e| DriverError::Failed(format!("decode {method}: {e}")))?;
        if reply.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(reply)
        } else {
            let err = reply
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown control error");
            Err(DriverError::Failed(format!("{method}: {err}")))
        }
    }
}

/// Configuration for one live-app connection.
#[derive(Debug, Clone)]
pub struct ControlChannelConfig {
    /// `127.0.0.1:<port>` of the running app's `--control` listener.
    pub addr: String,
    /// Bearer token (EVD-006: redacted in all output).
    pub token: Secret,
    /// Adapter-side base-document path (the saved `.pcraft`/PSD file of the
    /// document being carried). Read directly — the control protocol has no
    /// file-byte fetch.
    pub base_document: PathBuf,
    /// The app's `--automation-write-root` (where `app.save` may write).
    pub write_root: PathBuf,
    /// The app's `--automation-read-root` (where `app.open` reads).
    pub read_root: PathBuf,
}

/// Phase 2a driver: drives a **live** PhotoCraft desktop app.
pub struct ControlChannelDriver {
    client: ControlClient,
    cfg: ControlChannelConfig,
    /// Method-call transcript for evidence. The auth frame is never recorded;
    /// the token never appears here (EVD-006).
    transcript: Vec<serde_json::Value>,
}

impl std::fmt::Debug for ControlChannelDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `cfg` is Debug (its token is `Secret`, redacted); the raw client is
        // not shown. Nothing here can leak the token (EVD-006).
        f.debug_struct("ControlChannelDriver")
            .field("cfg", &self.cfg)
            .field("transcript_len", &self.transcript.len())
            .finish()
    }
}

impl ControlChannelDriver {
    pub fn connect(cfg: ControlChannelConfig) -> Result<Self, DriverError> {
        let client = ControlClient::connect(&cfg.addr, &cfg.token)?;
        Ok(ControlChannelDriver {
            client,
            cfg,
            transcript: Vec::new(),
        })
    }

    /// Evidence transcript: every whitelisted call with its outcome (results
    /// summarized by hash when large; no bytes, no tokens).
    pub fn transcript(&self) -> &[serde_json::Value] {
        &self.transcript
    }

    fn record(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        outcome: &Result<serde_json::Value, DriverError>,
    ) {
        let entry = match outcome {
            Ok(result) => serde_json::json!({
                "method": method,
                "params": params,
                "ok": true,
                "result": summarize(result),
            }),
            Err(e) => serde_json::json!({
                "method": method,
                "params": params,
                "ok": false,
                "error": e.to_string(),
            }),
        };
        self.transcript.push(entry);
    }

    fn call(
        &mut self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, DriverError> {
        let r = self.client.call(method, params);
        self.record(method, params, &r);
        r
    }

    fn execute(
        &mut self,
        command: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, DriverError> {
        self.call(
            "engine.execute",
            &serde_json::json!({ "command": command, "params": params }),
        )
    }

    /// One `engine.execute` (public: for harness setup commands).
    pub fn execute_command(
        &mut self,
        command: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, DriverError> {
        self.execute(command, params)
    }

    /// Write a scratch snapshot of the **current (dirty) state** of the active
    /// document WITHOUT touching it: duplicate the document, save the
    /// duplicate to a scratch `.pcraft`, read it back, delete the file, close
    /// the duplicate. The original's dirty flag, path and session position are
    /// untouched — a layered `app.save` marks the saved document clean and
    /// rebinds its path (`docs/control-protocol.md`), so saving the original
    /// would destroy the very "unsaved" state this carries. `image.duplicate`
    /// is the no-risk copy PhotoCraft's own agent docs recommend for
    /// experiments (`docs/mcp.md`). Desktop `app.save` has no format
    /// parameter; the path's `.pcraft` extension selects the native format.
    fn snapshot_scratch(&mut self, tag: &str) -> Result<Vec<u8>, DriverError> {
        let inspect = self.inspect()?;
        let docs_before = inspect
            .pointer("/session/documents")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let orig_active = inspect
            .pointer("/session/active")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        self.execute("image.duplicate", &serde_json::json!({}))?;
        // Fail closed unless the duplicate really is the active document:
        // app.save acts on the ACTIVE doc, and saving the original would
        // clean it (see above).
        let inspect = self.inspect()?;
        let docs_after = inspect
            .pointer("/session/documents")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let active_now = inspect
            .pointer("/session/active")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if docs_after != docs_before + 1 {
            return Err(DriverError::Failed(format!(
                "image.duplicate did not add a document ({docs_before} -> {docs_after})"
            )));
        }
        if active_now == orig_active || active_now as usize >= docs_after {
            return Err(DriverError::Failed(
                "image.duplicate did not activate the duplicate; refusing to app.save the original"
                    .into(),
            ));
        }
        let _ = std::fs::create_dir_all(self.cfg.write_root.join("carry"));
        // Name the scratch snapshot after the document so the destination shows
        // the carried document's own name after `app.open`. It lives in
        // `carry/`, never over the user's saved file.
        let doc_name = self
            .active_session_row()
            .ok()
            .and_then(|r| {
                r.get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| format!("carryon-{tag}"));
        let rel = format!("carry/{}.pcraft", sanitize_name(&doc_name));
        self.call("app.save", &serde_json::json!({ "path": rel }))?;
        let full = self.cfg.write_root.join(&rel);
        let bytes = std::fs::read(&full)
            .map_err(|e| DriverError::Failed(format!("read scratch snapshot {full:?}: {e}")))?;
        let _ = std::fs::remove_file(&full);
        // The duplicate is clean now, so closing it prompts nothing. Closing
        // returns the original to the active position.
        self.execute("file.close", &serde_json::json!({}))?;
        // Verify the original is active again; if not, reactivate it (fail
        // closed if that fails).
        let inspect = self.inspect()?;
        let active_end = inspect
            .pointer("/session/active")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if active_end != orig_active {
            self.call(
                "ui.menu.invoke",
                &serde_json::json!({ "id": format!("window.document.{orig_active}") }),
            )?;
        }
        Ok(bytes)
    }

    /// `session.inspect` (via `engine.execute`) → the active document row.
    fn active_session_row(&mut self) -> Result<serde_json::Value, DriverError> {
        let s = self.execute("session.inspect", &serde_json::json!({}))?;
        let active = s
            .get("active")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| DriverError::Failed("session.inspect: no active index".into()))?;
        s.get("documents")
            .and_then(|v| v.as_array())
            .and_then(|a| a.get(active as usize))
            .cloned()
            .ok_or_else(|| DriverError::Failed("session.inspect: active document missing".into()))
    }

    /// Walk the layer tree top-to-bottom, mapping the selected layers to
    /// portable `(name, occurrence)` targets (RESEARCH §1.3.2: occurrence is
    /// zero-based in document walk order).
    fn selected_layer_targets(tree: &serde_json::Value) -> Vec<crate::model::LayerTarget> {
        let mut selected = Vec::new();
        let mut name_counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        fn walk(
            node: &serde_json::Value,
            name_counts: &mut std::collections::HashMap<String, u32>,
            selected: &mut Vec<crate::model::LayerTarget>,
        ) {
            if let Some(layers) = node.as_array() {
                for l in layers {
                    let name = l.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let count = name_counts.entry(name.to_string()).or_insert(0);
                    let occurrence = *count;
                    *count += 1;
                    if l.get("selected").and_then(|v| v.as_bool()).unwrap_or(false) {
                        selected.push(crate::model::LayerTarget {
                            name: name.to_string(),
                            occurrence,
                        });
                    }
                    if let Some(children) = l.get("children") {
                        walk(children, name_counts, selected);
                    }
                }
            }
        }
        walk(tree, &mut name_counts, &mut selected);
        selected
    }

    /// Bring the live app to the carried workspace position. Must run after
    /// the carried document is open (activate first: engine commands act on
    /// the ACTIVE document).
    pub fn restore_live(&mut self, workspace: &Workspace) -> Result<(), DriverError> {
        // 1. Activate the carried tab (Window menu ids address open docs).
        self.call(
            "ui.menu.invoke",
            &serde_json::json!({ "id": format!("window.document.{}", workspace.active_document_index) }),
        )?;
        // 2. Tool (so later gestures use it).
        self.call(
            "ui.set",
            &serde_json::json!({ "tool": workspace.active_tool }),
        )?;
        // 3. Viewport.
        self.call(
            "ui.set",
            &serde_json::json!({
                "zoom": workspace.viewport.zoom,
                "center": workspace.viewport.center,
                "rotation": workspace.viewport.rotation,
            }),
        )?;
        // 4. Layer selection (last: uses the now-active document).
        for target in &workspace.selected_layers {
            self.execute(
                "layer.select",
                &serde_json::json!({
                    "name": target.name,
                    "occurrence": target.occurrence,
                    "mode": "replace",
                }),
            )?;
        }
        Ok(())
    }

    /// Open a document from the app's automation read root (`app.open`).
    /// Used by a destination host to open the carried document it has
    /// materialized under the app's read root.
    pub fn open_document(&mut self, rel_path: &str) -> Result<serde_json::Value, DriverError> {
        self.call("app.open", &serde_json::json!({ "path": rel_path }))
    }

    /// Save the active document to a path under the automation write root
    /// (`app.save {path}` — the *current*, possibly dirty, state).
    pub fn save_document(&mut self, rel_path: &str) -> Result<serde_json::Value, DriverError> {
        self.call("app.save", &serde_json::json!({ "path": rel_path }))
    }

    /// One `ui.set` (control method).
    pub fn set_ui(&mut self, fields: &serde_json::Value) -> Result<serde_json::Value, DriverError> {
        self.call("ui.set", fields)
    }

    /// Capture the app window to a PNG under the app's automation write root
    /// (`ui.screenshot {path}`).
    pub fn screenshot(&mut self, rel_path: &str) -> Result<serde_json::Value, DriverError> {
        self.call("ui.screenshot", &serde_json::json!({ "path": rel_path }))
    }

    /// Ask the app to quit gracefully (`app.quit`).
    pub fn quit(&mut self) -> Result<serde_json::Value, DriverError> {
        self.call("app.quit", &serde_json::json!({}))
    }

    /// One `ui.inspect` (public: used by the trait impl and by harnesses).
    pub fn inspect(&mut self) -> Result<serde_json::Value, DriverError> {
        self.call("ui.inspect", &serde_json::json!({}))
    }

    // ---- Stage-10 desktop UX: capture a live session / restore into one ------

    /// A compact, secret-free view of the connected app (the "live session"
    /// card in the Carry-On app). One `session.inspect` + one `ui.inspect`.
    pub fn live_summary(&mut self) -> Result<LiveSessionSummary, DriverError> {
        let row = self.active_session_row()?;
        let inspect = self.inspect()?;
        let active = row.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let document = inspect.get("document").cloned().unwrap_or_default();
        let selected = Self::selected_layer_targets(
            document.get("layers").unwrap_or(&serde_json::Value::Null),
        );
        let view = inspect
            .get("views")
            .and_then(|v| v.get(active as usize))
            .cloned()
            .unwrap_or_default();
        Ok(LiveSessionSummary {
            documents: inspect
                .pointer("/session/documents")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0),
            active,
            name: row
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            path: row
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            revision: row.get("revision").and_then(|v| v.as_u64()).unwrap_or(0),
            dirty: row.get("dirty").and_then(|v| v.as_bool()).unwrap_or(false),
            tool: inspect
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("move")
                .to_string(),
            zoom: view.get("zoom").and_then(|v| v.as_f64()).unwrap_or(1.0),
            rotation: view.get("rotation").and_then(|v| v.as_f64()).unwrap_or(0.0),
            selected_layers: selected.len(),
        })
    }

    /// Capture the live session into carryable state: the saved base document
    /// bytes (a `.pcraft` ZIP on this machine), the dirty-state snapshot (only
    /// when dirty; produced non-destructively with `image.duplicate` +
    /// `app.save` + `file.close`, see [`Self::snapshot_scratch`]), the
    /// ephemeral working position and the meta. With `split`, the base ZIP is
    /// also split into the parent-linked tile/blob bundle — the exact bytes
    /// PhotoCraft wrote, previews dropped (TILE-SPLITTING.md §2).
    ///
    /// The capture is a **point-in-time snapshot**: the caller creates the cut
    /// immediately after, so what the UI showed and what the cut seals are the
    /// same moment.
    pub fn capture(&mut self, split: bool) -> Result<LiveCapture, DriverError> {
        let document = self.document_bytes()?;
        let unsaved = self.unsaved_bytes()?;
        let meta = self.meta()?;
        let workspace: Workspace = serde_json::from_slice(&self.workspace_bytes()?)
            .map_err(|e| DriverError::Failed(format!("decode workspace: {e}")))?;
        let bundle = if split {
            Some(crate::zipbundle::split_pcraft_zip(&document).map_err(DriverError::Failed)?)
        } else {
            None
        };
        let parts = bundle.as_ref().map(|b| b.parts.len()).unwrap_or(0);
        let report = LiveCaptureReport {
            documents: 1,
            name: meta.document_name.clone(),
            revision: meta.revision,
            dirty: meta.dirty,
            path: meta.original_path.clone(),
            encoding: if split { "split" } else { "monolithic" }.into(),
            document_bytes: document.len() as u64,
            unsaved_bytes: unsaved.len() as u64,
            parts,
            total_bytes: (document.len() + unsaved.len()) as u64,
            tool: workspace.active_tool.clone(),
            zoom: workspace.viewport.zoom,
            rotation: workspace.viewport.rotation,
            selected_layers: workspace.selected_layers.len(),
        };
        Ok(LiveCapture {
            document,
            bundle,
            unsaved,
            workspace,
            meta,
            report,
        })
    }

    /// Restore carried state into the LIVE app behind this connection: write the
    /// payload under the app's automation read root (a `.pcraft` ZIP or a
    /// directory bundle — PhotoCraft opens both natively), `app.open` it, bring
    /// the working position back (tool, viewport, layer selection) on the NOW
    /// ACTIVE document, and optionally capture a screenshot for evidence.
    ///
    /// Prefers the dirty-state snapshot when one was carried: it is the exact
    /// current state (document + edits in one file), so the live app shows what
    /// the source showed. A clean carry opens the saved document itself.
    pub fn restore_into_live(
        &mut self,
        zip: Option<&[u8]>,
        bundle: Option<&DocumentBundle>,
        workspace: &Workspace,
        name_hint: &str,
        screenshot: Option<&str>,
    ) -> Result<LiveRestoreReport, DriverError> {
        let safe = sanitize_name(name_hint);
        let tag = crate::model::hex_sha256(
            zip.or_else(|| bundle.map(|b| b.manifest.as_slice()))
                .unwrap_or(b"empty"),
        );
        let tag = &tag[..8];
        // One subdirectory per carried capture: the opened document keeps its
        // own display name (`<name>.pcraft`), and repeated restores never
        // overwrite each other.
        let ingress = self.cfg.read_root.join("carryon-ingress").join(tag);
        std::fs::create_dir_all(&ingress)
            .map_err(|e| DriverError::Failed(format!("create ingress dir: {e}")))?;

        let rel = if let Some(bytes) = zip {
            let file = format!("{safe}.pcraft");
            std::fs::write(ingress.join(&file), bytes)
                .map_err(|e| DriverError::Failed(format!("write carried document: {e}")))?;
            format!("carryon-ingress/{tag}/{file}")
        } else if let Some(bundle) = bundle {
            let dir = format!("{safe}.pcraft");
            let full = ingress.join(&dir);
            let _ = std::fs::remove_dir_all(&full);
            for sub in ["tiles", "blobs"] {
                std::fs::create_dir_all(full.join(sub))
                    .map_err(|e| DriverError::Failed(format!("create bundle dir: {e}")))?;
            }
            for (path, bytes) in &bundle.parts {
                let (sub, name) = path
                    .split_once('/')
                    .ok_or_else(|| DriverError::Failed(format!("bad part path {path}")))?;
                std::fs::write(full.join(sub).join(name), bytes)
                    .map_err(|e| DriverError::Failed(format!("write part {path}: {e}")))?;
            }
            std::fs::write(full.join("manifest.json"), &bundle.manifest)
                .map_err(|e| DriverError::Failed(format!("write manifest: {e}")))?;
            format!("carryon-ingress/{tag}/{dir}")
        } else {
            return Err(DriverError::Failed(
                "restore: no carried document bytes (neither a snapshot nor a bundle)".into(),
            ));
        };

        self.call("app.open", &serde_json::json!({ "path": rel }))?;
        let active = {
            let inspect = self.inspect()?;
            inspect
                .pointer("/session/active")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32
        };
        // Working position on the now-active carried document. Activating by the
        // SOURCE's document index would be wrong here (the destination's window
        // order is its own); the opened document is already active.
        self.call(
            "ui.set",
            &serde_json::json!({
                "tool": workspace.active_tool,
                "zoom": workspace.viewport.zoom,
                "center": workspace.viewport.center,
                "rotation": workspace.viewport.rotation,
            }),
        )?;
        for target in &workspace.selected_layers {
            self.execute(
                "layer.select",
                &serde_json::json!({
                    "name": target.name,
                    "occurrence": target.occurrence,
                    "mode": "replace",
                }),
            )?;
        }
        // Evidence screenshot is best-effort: a backgrounded window may present
        // no frame and time out inside the app. The restore itself must not fail
        // because a screenshot did (the state proof is the engine oracle + the
        // independent inspect below).
        let mut shot = None;
        let mut shot_error = None;
        if let Some(path) = screenshot {
            let mut err = String::new();
            for attempt in 0..3 {
                match self.call("ui.screenshot", &serde_json::json!({ "path": path })) {
                    Ok(_) => {
                        shot = Some(path.to_string());
                        err.clear();
                        break;
                    }
                    Err(e) => {
                        err = e.to_string();
                        if attempt < 2 {
                            std::thread::sleep(Duration::from_millis(800));
                        }
                    }
                }
            }
            if shot.is_none() {
                shot_error = Some(err);
            }
        }
        let row = self.active_session_row()?;
        let inspect = self.inspect()?;
        let view = inspect
            .get("views")
            .and_then(|v| v.get(active as usize))
            .cloned()
            .unwrap_or_default();
        let selected = Self::selected_layer_targets(
            inspect
                .pointer("/document/layers")
                .unwrap_or(&serde_json::Value::Null),
        );
        Ok(LiveRestoreReport {
            opened_path: rel,
            active,
            name: row
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            revision: row.get("revision").and_then(|v| v.as_u64()).unwrap_or(0),
            dirty: row.get("dirty").and_then(|v| v.as_bool()).unwrap_or(false),
            tool: inspect
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            zoom: view.get("zoom").and_then(|v| v.as_f64()).unwrap_or(0.0),
            rotation: view.get("rotation").and_then(|v| v.as_f64()).unwrap_or(0.0),
            selected_layers: selected.len(),
            screenshot: shot,
            screenshot_error: shot_error,
        })
    }

    /// Apply the carried working position to the CURRENT active document — the
    /// destination-side `session.restore` step (the carried document is already
    /// open and active; no source index is trusted).
    pub fn restore_live_current(&mut self, workspace: &Workspace) -> Result<(), DriverError> {
        self.call(
            "ui.set",
            &serde_json::json!({
                "tool": workspace.active_tool,
                "zoom": workspace.viewport.zoom,
                "center": workspace.viewport.center,
                "rotation": workspace.viewport.rotation,
            }),
        )?;
        for target in &workspace.selected_layers {
            self.execute(
                "layer.select",
                &serde_json::json!({
                    "name": target.name,
                    "occurrence": target.occurrence,
                    "mode": "replace",
                }),
            )?;
        }
        Ok(())
    }
}

/// `LiveCapture` report: sizes and session facts only, no bytes, no token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveCaptureReport {
    pub documents: usize,
    pub name: String,
    pub revision: u64,
    pub dirty: bool,
    pub path: Option<String>,
    pub encoding: String,
    pub document_bytes: u64,
    pub unsaved_bytes: u64,
    pub parts: usize,
    pub total_bytes: u64,
    pub tool: String,
    pub zoom: f64,
    pub rotation: f64,
    pub selected_layers: usize,
}

/// Captured live session (source side) — everything a cut needs.
pub struct LiveCapture {
    /// Saved base document bytes (`.pcraft` ZIP).
    pub document: Vec<u8>,
    /// Split encoding of `document` when requested (`None` = monolithic).
    pub bundle: Option<DocumentBundle>,
    /// Dirty-state snapshot ZIP (`image.duplicate` + `app.save`; empty when clean).
    pub unsaved: Vec<u8>,
    pub workspace: Workspace,
    pub meta: PhotocraftMeta,
    pub report: LiveCaptureReport,
}

/// Restored live session (destination side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveRestoreReport {
    pub opened_path: String,
    pub active: u32,
    pub name: String,
    pub revision: u64,
    pub dirty: bool,
    pub tool: String,
    pub zoom: f64,
    pub rotation: f64,
    pub selected_layers: usize,
    pub screenshot: Option<String>,
    /// Why the evidence screenshot is absent, when it is (never a restore failure).
    pub screenshot_error: Option<String>,
}

/// Compact live-session card (source or destination), token-free.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionSummary {
    pub documents: usize,
    pub active: u32,
    pub name: String,
    pub path: Option<String>,
    pub revision: u64,
    pub dirty: bool,
    pub tool: String,
    pub zoom: f64,
    pub rotation: f64,
    pub selected_layers: usize,
}

/// Keep an app-provided document name safe for a file under the ingress dir:
/// no path separators, no traversal, bounded length, always non-empty.
fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').trim_matches('_');
    let stem = trimmed.strip_suffix(".pcraft").unwrap_or(trimmed);
    let out = if stem.is_empty() { "carried" } else { stem };
    out.chars().take(64).collect()
}

/// Keep transcript entries small: big results become a sha256 + length.
fn summarize(v: &serde_json::Value) -> serde_json::Value {
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.len() <= 2048 {
        v.clone()
    } else {
        serde_json::json!({
            "summary": format!("{} bytes, sha256 {}", s.len(), crate::model::hex_sha256(s.as_bytes())),
        })
    }
}

impl PhotocraftDriver for ControlChannelDriver {
    fn document_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        // The saved base document: the configured path, else the live document's
        // own saved path as the app reports it (`session.inspect` → `path`,
        // relative to the automation read root). Reading the user's selected
        // document is the adapter's declared `read_selected_workspace` permission.
        let mut candidates = vec![self.cfg.base_document.clone()];
        if let Ok(row) = self.active_session_row() {
            if let Some(p) = row.get("path").and_then(|v| v.as_str()) {
                let path = Path::new(p);
                candidates.push(if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    self.cfg.read_root.join(path)
                });
            }
        }
        for path in candidates {
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(bytes);
            }
        }
        Err(DriverError::Failed(
            "the live document has no readable saved base (save it in PhotoCraft first: \
             a carry needs the document object)"
                .into(),
        ))
    }

    fn unsaved_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        let row = self.active_session_row()?;
        let dirty = row
            .get("dirty")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| DriverError::Failed("session row: no dirty flag".into()))?;
        if !dirty {
            return Ok(Vec::new());
        }
        self.snapshot_scratch("unsaved")
    }

    fn meta(&mut self) -> Result<PhotocraftMeta, DriverError> {
        let row = self.active_session_row()?;
        let revision = row
            .get("revision")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| DriverError::Failed("session row: no revision".into()))?;
        let dirty = row.get("dirty").and_then(|v| v.as_bool()).unwrap_or(false);
        Ok(PhotocraftMeta {
            schema_id: "carryon.photocraft.session".into(),
            schema_version: 1,
            adapter_version: env!("CARGO_PKG_VERSION").into(),
            original_path: row
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            document_name: row
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            revision,
            // The live control surface does not expose saved_revision; 0 =
            // unknown (see model::PhotocraftMeta).
            saved_revision: if dirty { 0 } else { revision },
            dirty,
            photocraft_pin: crate::model::PHOTOCRAFT_PIN.into(),
            unsaved_mode: UnsavedMode::Snapshot,
            // The control channel serves the monolithic zip only (a split
            // bundle would need a directory-bundle save; TILE-SPLITTING.md §7).
            document_encoding: crate::model::DocumentEncoding::Monolithic,
        })
    }

    fn workspace_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        let inspect = self.inspect()?;
        let tool = inspect
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("move")
            .to_string();
        let session = inspect.get("session").cloned().unwrap_or_default();
        let active = session.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let view = inspect
            .get("views")
            .and_then(|v| v.get(active as usize))
            .cloned()
            .unwrap_or_default();
        let zoom = view.get("zoom").and_then(|v| v.as_f64()).unwrap_or(1.0);
        let center = view
            .get("center")
            .and_then(|v| v.as_array())
            .map(|a| {
                [
                    a.first().and_then(|x| x.as_f64()).unwrap_or(0.0),
                    a.get(1).and_then(|y| y.as_f64()).unwrap_or(0.0),
                ]
            })
            .unwrap_or([0.0, 0.0]);
        let rotation = view.get("rotation").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let doc = inspect.get("document").cloned().unwrap_or_default();
        let selected_layers =
            Self::selected_layer_targets(doc.get("layers").unwrap_or(&serde_json::Value::Null));
        let workspace = Workspace {
            active_document_index: active,
            selected_layers,
            viewport: Viewport {
                zoom,
                center,
                rotation,
            },
            active_tool: tool,
            panels: inspect
                .get("panels")
                .cloned()
                .unwrap_or(serde_json::json!({})),
        };
        serde_json::to_vec(&workspace)
            .map_err(|e| DriverError::Failed(format!("encode workspace: {e}")))
    }

    fn apply_steps(&mut self, steps: &[Step]) -> Result<EditOutcome, DriverError> {
        let mut ran = 0usize;
        for step in steps {
            match self.execute(&step.id, &step.params) {
                Ok(_) => ran += 1,
                Err(e) => {
                    // PhotoCraft batches are not atomic and report partial
                    // application (RESEARCH §2.4): report what ran, fail
                    // closed with the error.
                    return Err(DriverError::Failed(format!(
                        "step {ran} ({}) failed after {ran} applied: {e}",
                        step.id
                    )));
                }
            }
        }
        let row = self.active_session_row()?;
        let revision = row.get("revision").and_then(|v| v.as_u64()).unwrap_or(0);
        // Hash the post-edit dirty state by snapshotting (scratch file, then
        // delete) — the same oracle basis `document.edit` reports.
        let dirty = row.get("dirty").and_then(|v| v.as_bool()).unwrap_or(false);
        let unsaved_hash = if dirty {
            let bytes = self.snapshot_scratch("edit-hash")?;
            crate::model::hex_sha256(&bytes)
        } else {
            crate::model::hex_sha256(&[])
        };
        Ok(EditOutcome {
            steps_ran: ran,
            new_revision: revision,
            unsaved_hash,
        })
    }

    fn restore(&mut self, workspace: &Workspace) -> Result<(), DriverError> {
        // Destination-side `session.restore`: the carried document is already
        // open and active in this app; apply the working position to the ACTIVE
        // document (a source document index means nothing in this window).
        self.restore_live_current(workspace)
    }
}

impl DriverError {}

/// Serde shape for tests / harness configuration files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlChannelConfigFile {
    pub addr: String,
    pub token_file: String,
    pub base_document: String,
    pub write_root: String,
    pub read_root: String,
}

impl ControlChannelConfigFile {
    /// Load with the token read from its 0600 file (never logged; EVD-006).
    pub fn into_config(self) -> Result<ControlChannelConfig, DriverError> {
        let token = std::fs::read_to_string(&self.token_file)
            .map_err(|e| DriverError::Failed(format!("read token file: {e}")))?;
        Ok(ControlChannelConfig {
            addr: self.addr,
            token: Secret::new(token.trim()),
            base_document: Path::new(&self.base_document).to_path_buf(),
            write_root: Path::new(&self.write_root).to_path_buf(),
            read_root: Path::new(&self.read_root).to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal in-process mock of the PhotoCraft control server speaking
    /// the documented JSON-lines protocol, so the client logic (auth,
    /// whitelist, framing, error mapping) is tested without a live app.
    struct MockServer {
        addr: std::net::SocketAddr,
        /// Held so the serving thread lives as long as the test; the thread
        /// exits when the client closes the connection.
        #[allow(dead_code)]
        handle: std::thread::JoinHandle<()>,
    }

    impl MockServer {
        fn start(token: &str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let token = token.to_string();
            let handle = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
                    let id = frame.get("id").cloned().unwrap_or_default();
                    let method = frame.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    let reply = if method == "auth" {
                        let got = frame
                            .pointer("/params/token")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if got == token {
                            serde_json::json!({"id": id, "ok": true, "result": {}})
                        } else {
                            serde_json::json!({"id": id, "ok": false, "error": "bad token"})
                        }
                    } else {
                        serde_json::json!({
                            "id": id, "ok": true,
                            "result": {"echoMethod": method, "params": frame.get("params").cloned().unwrap_or_default()}
                        })
                    };
                    let mut out = serde_json::to_string(&reply).unwrap();
                    out.push('\n');
                    if writer.write_all(out.as_bytes()).is_err() {
                        return;
                    }
                }
            });
            MockServer { addr, handle }
        }
    }

    fn test_cfg(addr: &std::net::SocketAddr) -> ControlChannelConfig {
        ControlChannelConfig {
            addr: addr.to_string(),
            token: Secret::new("t".repeat(64)),
            base_document: PathBuf::from("/nonexistent/base.pcraft"),
            write_root: std::env::temp_dir(),
            read_root: std::env::temp_dir(),
        }
    }

    #[test]
    fn secret_debug_and_display_are_redacted() {
        let s = Secret::new("super-secret-token-value");
        assert!(!format!("{s:?}").contains("super-secret"));
        assert!(!format!("{s}").contains("super-secret"));
        assert_eq!(s.as_str(), "super-secret-token-value");
    }

    #[test]
    fn auth_success_and_method_calls_round_trip() {
        let server = MockServer::start(&"t".repeat(64));
        let mut d = ControlChannelDriver::connect(test_cfg(&server.addr)).unwrap();
        let r = d.call("ui.inspect", &serde_json::json!({})).unwrap();
        assert_eq!(r["echoMethod"], "ui.inspect");
        // Transcript excludes auth and never contains the token.
        let t = serde_json::to_string(d.transcript()).unwrap();
        assert!(!t.contains("auth"));
        assert!(!t.contains("ttttt"));
        assert!(t.contains("ui.inspect"));
    }

    #[test]
    fn bad_token_fails_closed() {
        let server = MockServer::start("correct-token");
        let mut cfg = test_cfg(&server.addr);
        cfg.token = Secret::new("wrong-token");
        assert!(ControlChannelDriver::connect(cfg).is_err());
    }

    #[test]
    fn non_whitelisted_method_fails_before_any_io() {
        let server = MockServer::start(&"t".repeat(64));
        let mut d = ControlChannelDriver::connect(test_cfg(&server.addr)).unwrap();
        let err = d
            .call("file.delete_everything", &serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("whitelisted"), "{err}");
        // No request reached the server: the transcript records only the
        // refusal itself (ok:false + the whitelist error), nothing echoed.
        let t = serde_json::to_string(d.transcript()).unwrap();
        assert!(t.contains("whitelisted"), "refusal must be recorded: {t}");
        assert!(
            !t.contains("echoMethod"),
            "no server round trip may appear: {t}"
        );
    }

    #[test]
    fn document_bytes_reads_base_file() {
        let server = MockServer::start(&"t".repeat(64));
        let mut cfg = test_cfg(&server.addr);
        let tmp = tempfile_dir();
        let base = tmp.join("base.pcraft");
        std::fs::write(&base, b"base-bytes").unwrap();
        cfg.base_document = base;
        let mut d = ControlChannelDriver::connect(cfg).unwrap();
        assert_eq!(
            PhotocraftDriver::document_bytes(&mut d).unwrap(),
            b"base-bytes"
        );
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn selected_layer_targets_maps_names_with_occurrence() {
        let tree = serde_json::json!([
            {"id": 1, "name": "A", "selected": true},
            {"id": 2, "name": "A", "selected": false},
            {"id": 3, "name": "B", "selected": true, "children": [
                {"id": 4, "name": "A", "selected": true}
            ]}
        ]);
        let targets = ControlChannelDriver::selected_layer_targets(&tree);
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].name, "A");
        assert_eq!(targets[0].occurrence, 0);
        assert_eq!(targets[1].name, "B");
        assert_eq!(targets[1].occurrence, 0);
        assert_eq!(targets[2].name, "A");
        assert_eq!(targets[2].occurrence, 2); // third "A" in walk order
    }

    #[test]
    fn summarize_redacts_large_results() {
        let big = serde_json::json!({"data": "x".repeat(5000)});
        let s = summarize(&big);
        let text = serde_json::to_string(&s).unwrap();
        assert!(
            text.contains("sha256"),
            "large results must be hashed: {text}"
        );
        assert!(!text.contains("xxxx"));
    }

    fn tempfile_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("pcraft-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }
}
