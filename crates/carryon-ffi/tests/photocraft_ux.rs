//! ABI-level test for the stage-10 PhotoCraft UX entries against a **mock**
//! PhotoCraft control server (protocol-level: auth, session.inspect,
//! image.duplicate, app.save, file.close, ui.inspect, ui.set, layer.select).
//!
//! This pins the C surface (config parsing, capture registration, fail-closed
//! restore) without a GUI app. The real two-app run is
//! `examples/photocraft_ux_live.rs` + `docs/artcraft-bridge/scripts/photocraft_ux_local.sh`
//! (OBSERVED, LOCAL).

use std::ffi::{c_char, CStr, CString};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TOKEN: &str = "test-token-0123456789";

/// Serve the documented JSON-lines control protocol until the process exits.
fn mock_server(read_root: PathBuf, write_root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let state = Arc::new(Mutex::new(1usize)); // documents; the duplicate adds one
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { break };
            let read = stream.try_clone().unwrap();
            let mut reader = BufReader::new(read);
            let mut writer = stream;
            let state = state.clone();
            let read_root = read_root.clone();
            let write_root = write_root.clone();
            std::thread::spawn(move || loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
                let id = frame.get("id").cloned().unwrap_or_default();
                let method = frame.get("method").and_then(|m| m.as_str()).unwrap_or("");
                let params = frame.get("params").cloned().unwrap_or_default();
                let reply = match method {
                    "auth" => {
                        if params.get("token").and_then(|v| v.as_str()) == Some(TOKEN) {
                            serde_json::json!({"id": id, "ok": true, "result": {}})
                        } else {
                            serde_json::json!({"id": id, "ok": false, "error": "bad token"})
                        }
                    }
                    "engine.execute" => {
                        let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
                        match command {
                            "session.inspect" => {
                                let docs = *state.lock().unwrap();
                                let documents: Vec<serde_json::Value> = (0..docs)
                                    .map(|i| {
                                        serde_json::json!({
                                            "index": i, "name": "mock.pcraft", "path": "mock.pcraft",
                                            "revision": 2, "dirty": true, "width": 10, "height": 10,
                                        })
                                    })
                                    .collect();
                                serde_json::json!({
                                    "id": id, "ok": true,
                                    "result": {"active": docs - 1, "documents": documents}
                                })
                            }
                            "image.duplicate" => {
                                *state.lock().unwrap() += 1;
                                serde_json::json!({"id": id, "ok": true, "result": {}})
                            }
                            "file.close" => {
                                let mut s = state.lock().unwrap();
                                *s = (*s).saturating_sub(1).max(1);
                                serde_json::json!({"id": id, "ok": true, "result": {}})
                            }
                            _ => serde_json::json!({"id": id, "ok": true, "result": {}}),
                        }
                    }
                    "ui.inspect" => {
                        let docs = *state.lock().unwrap();
                        let documents: Vec<serde_json::Value> =
                            (0..docs).map(|i| serde_json::json!({"index": i})).collect();
                        serde_json::json!({
                            "id": id, "ok": true,
                            "result": {
                                "tool": "brush",
                                "session": {"active": docs - 1, "documents": documents},
                                "views": [{"zoom": 1.5, "center": [10.0, 20.0], "rotation": 3.0}],
                                "document": {"layers": [{"name": "Background", "selected": true}]},
                                "panels": {},
                            }
                        })
                    }
                    "app.save" => {
                        let rel = params
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("x.pcraft");
                        let target = write_root.join(rel);
                        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                        std::fs::write(&target, mock_zip()).unwrap();
                        serde_json::json!({"id": id, "ok": true, "result": {"path": rel}})
                    }
                    "app.open" | "ui.set" | "layer.select" | "ui.screenshot" | "ui.menu.invoke" => {
                        serde_json::json!({"id": id, "ok": true, "result": {}})
                    }
                    other => {
                        serde_json::json!({"id": id, "ok": false, "error": format!("unknown method {other}")})
                    }
                };
                let mut out = serde_json::to_string(&reply).unwrap();
                out.push('\n');
                if writer.write_all(out.as_bytes()).is_err() {
                    return;
                }
                // The server is one-shot per connection for our purposes; keep reading.
                let _ = &read_root;
            });
        }
    });
    addr
}

/// A minimal **stored** `.pcraft` ZIP: manifest + two tiles + a preview.
fn mock_zip() -> Vec<u8> {
    const LOCAL: &[u8; 4] = b"PK\x03\x04";
    const CENTRAL: &[u8; 4] = b"PK\x01\x02";
    const EOCD: &[u8; 4] = b"PK\x05\x06";
    let hex = |c: char| -> String { std::iter::repeat_n(c, 64).collect() };
    let t1 = format!("tiles/{}.zst", hex('a'));
    let t2 = format!("tiles/{}.zst", hex('b'));
    let entries: Vec<(&str, &[u8])> = vec![
        ("manifest.json", br#"{"version":1}"#),
        ("thumb.png", b"thumb"),
        (t1.as_str(), b"tile-a-bytes"),
        (t2.as_str(), b"tile-b-bytes"),
    ];
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in &entries {
        let off = out.len();
        let name_b = name.as_bytes();
        out.extend_from_slice(LOCAL);
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_b);
        out.extend_from_slice(data);
        central.extend_from_slice(CENTRAL);
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&(off as u32).to_le_bytes());
        central.extend_from_slice(name_b);
    }
    let cd_off = out.len();
    out.extend_from_slice(&central);
    out.extend_from_slice(EOCD);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&(cd_off as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

unsafe fn take(ptr: *mut c_char) -> String {
    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    carryon_ffi::strings::carryon_string_free(ptr);
    s
}

#[test]
fn inspect_and_register_over_the_mock_control_channel() {
    let dir = tempfile::tempdir().unwrap();
    let read_root = dir.path().join("read");
    let write_root = dir.path().join("write");
    std::fs::create_dir_all(&read_root).unwrap();
    std::fs::create_dir_all(&write_root).unwrap();
    let base = read_root.join("mock.pcraft");
    std::fs::write(&base, mock_zip()).unwrap();
    let addr = mock_server(read_root.clone(), write_root.clone());

    // The destination data dir for the engine handle.
    let data = dir.path().join("core");
    std::fs::create_dir_all(&data).unwrap();

    let cfg = serde_json::json!({
        "addr": addr,
        "token": TOKEN, // test-only inline token; the app uses token_file (EVD-006)
        "read_root": read_root.to_str().unwrap(),
        "write_root": write_root.to_str().unwrap(),
        "encoding": "split",
        "session_title": "mock carry",
    })
    .to_string();
    let cfg_c = CString::new(cfg).unwrap();

    unsafe {
        // ---- inspect --------------------------------------------------------
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = carryon_ffi::abi_photocraft::carryon_photocraft_inspect(cfg_c.as_ptr(), &mut out);
        assert_eq!(rc, 0, "inspect rc");
        let json: serde_json::Value = serde_json::from_str(&take(out)).unwrap();
        assert_eq!(json["live"], true);
        assert_eq!(json["name"], "mock.pcraft");
        assert_eq!(json["dirty"], true);
        assert_eq!(json["tool"], "brush");
        // The token must never appear in the output (EVD-006).
        assert!(!json.to_string().contains(TOKEN));

        // ---- register (capture) --------------------------------------------
        let data_c = CString::new(data.to_str().unwrap()).unwrap();
        let core = carryon_ffi::abi_core::carryon_core_open(data_c.as_ptr());
        assert!(!core.is_null(), "core open");
        let mut out2: *mut c_char = std::ptr::null_mut();
        let rc2 = carryon_ffi::abi_photocraft::carryon_photocraft_register(
            core,
            cfg_c.as_ptr(),
            &mut out2,
        );
        if rc2 != 0 {
            let mut len: usize = 0;
            let _ = carryon_ffi::carryon_last_error(std::ptr::null_mut(), &mut len);
            let mut buf = vec![0u8; len.max(2)];
            let _ = carryon_ffi::carryon_last_error(buf.as_mut_ptr(), &mut len);
            panic!(
                "register rc={rc2}: {}",
                String::from_utf8_lossy(&buf[..len.saturating_sub(1)])
            );
        }
        let cap: serde_json::Value = serde_json::from_str(&take(out2)).unwrap();
        assert_eq!(cap["encoding"], "split");
        assert_eq!(cap["parts"], 2, "the two tile entries, previews dropped");
        assert!(cap["adapter_id"]
            .as_str()
            .unwrap()
            .starts_with("org.carryon.photocraft"));
        assert!(
            cap["unsaved_bytes"].as_u64().unwrap() > 0,
            "dirty snapshot captured"
        );
        assert!(!cap.to_string().contains(TOKEN));

        // ---- restore on an unknown cut fails closed --------------------------
        let mirror = CString::new("8a1a1b6c-0000-4000-8000-000000000000").unwrap();
        let mut out3: *mut c_char = std::ptr::null_mut();
        let rc3 = carryon_ffi::abi_photocraft::carryon_photocraft_restore(
            core,
            cfg_c.as_ptr(),
            mirror.as_ptr(),
            0,
            &mut out3,
        );
        assert_ne!(rc3, 0, "restore of an unsealed cut must fail closed");

        carryon_ffi::abi_core::carryon_core_free(core);
    }

    // The capture wrote the dirty snapshot into the app's write root (scratch,
    // removed after reading) and split the base ZIP into two tile objects.
    let _ = Path::new(&write_root);
}
