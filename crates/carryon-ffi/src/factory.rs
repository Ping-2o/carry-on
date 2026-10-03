//! Adapter-by-id factory — the honest "import a program" entry point (spec §3.8).
//!
//! C cannot construct a `Box<dyn Adapter>`. Instead it names one of the fixed,
//! compiled-in adapters by id and passes JSON parameters (data, never behavior). An
//! unknown id is refused. There is no path by which a caller injects executable code
//! or selects a program to run (§3.8 no remote code execution).

use carryon_adapter_api::Adapter;
use carryon_core::CoreError;
use serde_json::Value;

/// Build one of the fixed compiled-in adapters from an id + JSON params.
///
/// | id                       | params                                                   |
/// |--------------------------|----------------------------------------------------------|
/// | `org.carryon.file`       | `{ "path": "/abs" }` (reads disk, L1)                    |
/// | `org.carryon.launcher`   | `{ "uri": "..." }` \| `{ "file": "/abs" }` \| `{}`       |
/// | `org.carryon.graph`      | `{ "sample": true }` \| `{ "nodes":[…], "edges":[…] }`   |
/// | `org.carryon.image`      | `{ "sample": true }` \| `{ "width":W,"height":H,"cells":[…] }` |
/// | `org.carryon.editor`     | `{ "sample": true }` \| `{ "session":"…","text":"…" }` (L4) |
/// | `org.carryon.unsupported`| `{}`                                                     |
pub fn build_adapter(adapter_id: &str, params: &Value) -> Result<Box<dyn Adapter>, CoreError> {
    use carryon_core::error::{AdapterCode, SchemaCode};
    let bad = |m: &str| CoreError::schema(SchemaCode::Invalid, m.to_string());

    match adapter_id {
        "org.carryon.file" => {
            if let Some(path) = params.get("path").and_then(|v| v.as_str()) {
                let a = carryon_adapter_file::FileAdapter::from_path(std::path::Path::new(path))
                    .map_err(|e| {
                        CoreError::schema(SchemaCode::Invalid, format!("file open: {e}"))
                    })?;
                Ok(Box::new(a))
            } else if let Some(uri) = params.get("uri").and_then(|v| v.as_str()) {
                // Inline bytes form: UTF-8 content passed directly (no base64 dep).
                let bytes = params
                    .get("bytes_utf8")
                    .and_then(|v| v.as_str())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                Ok(Box::new(carryon_adapter_file::FileAdapter::new(uri, bytes)))
            } else {
                Err(bad("file adapter needs 'path' or 'uri'"))
            }
        }
        "org.carryon.launcher" => {
            use carryon_adapter_launcher::{LaunchTarget, LauncherAdapter};
            let target = if let Some(uri) = params.get("uri").and_then(|v| v.as_str()) {
                LaunchTarget::uri(uri)
            } else if let Some(file) = params.get("file").and_then(|v| v.as_str()) {
                LaunchTarget::file(file)
            } else {
                LaunchTarget::uri("about:blank")
            };
            Ok(Box::new(LauncherAdapter::new(target)))
        }
        "org.carryon.graph" => {
            use carryon_adapter_graph::{Edge, GraphAdapter, Node};
            if params.get("sample").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(Box::new(GraphAdapter::sample()));
            }
            let nodes: Vec<Node> = params
                .get("nodes")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| bad(&format!("graph nodes: {e}")))?
                .unwrap_or_default();
            let edges: Vec<Edge> = params
                .get("edges")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| bad(&format!("graph edges: {e}")))?
                .unwrap_or_default();
            if nodes.is_empty() {
                return Err(bad("graph adapter needs 'sample' or non-empty 'nodes'"));
            }
            Ok(Box::new(GraphAdapter::new(nodes, edges)))
        }
        "org.carryon.image" => {
            use carryon_adapter_image::{HeightField, ImageAdapter};
            if params.get("sample").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(Box::new(ImageAdapter::sample()));
            }
            let field: HeightField = serde_json::from_value(params.clone())
                .map_err(|e| bad(&format!("image field: {e}")))?;
            Ok(Box::new(ImageAdapter::new(field)))
        }
        "org.carryon.editor" => {
            use carryon_adapter_editor::{EditorAdapter, Navigation};
            // Cooperative editor (L3 structured session + L4 authority).
            //   { "sample": true }            -> short back-compat sample
            //   { "session_v1": true }        -> recognizable L3 demo session
            //   { "session", "text" }         -> back-compat single-document session
            //   { "session", "document", "unsaved", "cursor", "selection":[a,h],
            //     "viewport":[x,y], "active_tab" } -> full structured session
            if params.get("sample").and_then(|v| v.as_bool()) == Some(true) {
                Ok(Box::new(EditorAdapter::sample()))
            } else if params.get("session_v1").and_then(|v| v.as_bool()) == Some(true) {
                Ok(Box::new(EditorAdapter::session_v1()))
            } else if params.get("document").is_some() || params.get("unsaved").is_some() {
                let session = params
                    .get("session")
                    .and_then(|v| v.as_str())
                    .unwrap_or("editor-session");
                let document = params
                    .get("document")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec();
                let unsaved = params
                    .get("unsaved")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec();
                let sel = params.get("selection").and_then(|v| v.as_array());
                let vp = params.get("viewport").and_then(|v| v.as_array());
                let u = |a: Option<&Vec<Value>>, i: usize| {
                    a.and_then(|a| a.get(i))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                };
                let nav = Navigation {
                    cursor: params.get("cursor").and_then(|v| v.as_u64()).unwrap_or(0),
                    selection_anchor: u(sel, 0),
                    selection_head: u(sel, 1),
                    scroll_x: u(vp, 0),
                    scroll_y: u(vp, 1),
                    active_tab: params
                        .get("active_tab")
                        .and_then(|v| v.as_str())
                        .unwrap_or("main")
                        .to_string(),
                };
                Ok(Box::new(EditorAdapter::with_session(
                    session, document, unsaved, nav,
                )))
            } else {
                let session = params
                    .get("session")
                    .and_then(|v| v.as_str())
                    .unwrap_or("editor-session");
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                Ok(Box::new(EditorAdapter::new(
                    session,
                    text.as_bytes().to_vec(),
                )))
            }
        }
        "org.carryon.unsupported" => Ok(Box::new(carryon_adapter_unsupported::UnsupportedAdapter)),
        other => Err(CoreError::adapter(
            AdapterCode::Missing,
            format!("unknown adapter id '{other}' (not a compiled-in adapter)"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_known_ids() {
        assert!(build_adapter("org.carryon.graph", &serde_json::json!({"sample":true})).is_ok());
        assert!(build_adapter("org.carryon.image", &serde_json::json!({"sample":true})).is_ok());
        assert!(build_adapter("org.carryon.launcher", &serde_json::json!({"uri":"x"})).is_ok());
        assert!(build_adapter("org.carryon.unsupported", &serde_json::json!({})).is_ok());
    }

    #[test]
    fn unknown_id_refused() {
        match build_adapter("com.evil.rce", &serde_json::json!({})) {
            Err(e) => assert_eq!(e.family(), "ADAPTER"),
            Ok(_) => panic!("unknown id must be refused"),
        }
    }
}
