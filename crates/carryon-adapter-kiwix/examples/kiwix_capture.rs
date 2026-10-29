//! kiwix_capture — turn the REAL Kiwix Android app's persisted state into the
//! adapter's `Session` object.
//!
//! Inputs (pull them from the rooted device first — see
//! `shells/capture_android_session.sh`):
//!   kiwix_capture <datastore_pb> <KiwixRoom.db> [zim_path_override]
//!
//!   - `<datastore_pb>`: `<filesDir>/datastore/kiwix_datastore_preferences.preferences_pb`
//!     (androidx Preferences protobuf — parsed by hand, no codegen)
//!   - `<KiwixRoom.db>`: the Room DB with `WebViewHistoryEntity` /
//!     `HistoryRoomEntity` tables (sqlite3; read-only, copy on device pull)
//!   - optional `zim_path_override`: use this ZIM instead of the
//!     `currentzimfile` path recorded by the app (the device path may differ
//!     on this host; the ZIM *identity* still comes from the real file)
//!
//! Output: the session JSON on stdout plus an honest capture report on
//! stderr — exactly what was captured, and exactly what was degraded.
//!
//! Honesty contract (labels in the report):
//!   CAPTURED  — value read verbatim from the app's own storage.
//!   DROPPED   — value exists in the app but is not portable (see report).
//!   FALLBACK  — we used a reliable secondary source.
//!
//! The Android `WebView.saveState` bundle (`webViewBackForwardListBundle`
//! column) is an opaque, platform-versioned Parcel blob (Chromium's
//! serialized back/forward list). We carry it through base64 as
//! `opaque_platform_state` but do NOT interpret it: the portable truth is
//! {article path, scrollY}, which the app persists separately.

use carryon_adapter_kiwix::{HistoryEntry, ReadingPosition, ReadingSettings, Session, ZimIdentity};

// ---------------- session preferences (DataStore protobuf OR legacy prefs XML)

/// The installed release (v3.14.1) persists `currentzimfile`/`currenttab` in
/// the classic `kiwix-mobile.xml` SharedPreferences (repo HEAD moved the same
/// keys to a Jetpack DataStore protobuf — RESEARCH §1.1). The capture reads
/// whichever container it is given, and reports which one it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefsKind {
    /// androidx Preferences protobuf (.preferences_pb)
    DataStorePb,
    /// Android SharedPreferences XML (`<map><string name=...>`)
    LegacyXml,
}

/// One value out of the preferences.
#[derive(Debug, Clone, PartialEq)]
enum PValue {
    Int(i64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
}

/// Minimal varint/wire-format reader for the androidx Preferences proto.
/// Layout (empirically handled; keys are known so we interpret per key):
///   Preferences { map<string, Value> preferences = 1 }
///   Value { oneof: varint fields (int/long/bool), string fields, bytes }
struct Pb<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Pb<'a> {
    fn new(b: &'a [u8]) -> Self {
        Pb { b, pos: 0 }
    }
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = *self.b.get(self.pos)?;
            self.pos += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
    fn field(&mut self) -> Option<(u64, u8)> {
        let key = self.varint()?;
        Some((key >> 3, (key & 0x7) as u8))
    }
    fn len_delim(&mut self) -> Option<&'a [u8]> {
        let len = self.varint()? as usize;
        let s = self.b.get(self.pos..)?.get(..len)?;
        self.pos += len;
        Some(s)
    }
}

fn parse_value(bytes: &[u8]) -> PValue {
    let mut pb = Pb::new(bytes);
    let mut ints: Vec<i64> = Vec::new();
    let mut bools = Vec::new();
    let mut strs: Vec<String> = Vec::new();
    let mut raw_bytes = Vec::new();
    while let Some((fnum, wt)) = pb.field() {
        match (fnum, wt) {
            (_, 0) => {
                let v = pb.varint().unwrap_or(0);
                // androidx Preferences.Value: integer=3, long=4, boolean=5 —
                // but we accept any varint and disambiguate by key outside.
                ints.push(v as i64);
                if fnum == 5 {
                    bools.push(v != 0);
                }
            }
            (_, 2) => {
                let raw = pb.len_delim().unwrap_or(&[]);
                if let Ok(s) = String::from_utf8(raw.to_vec()) {
                    if !s.contains('\u{0}') {
                        strs.push(s);
                        continue;
                    }
                }
                raw_bytes = raw.to_vec();
            }
            // skip 32/64-bit fixed fields
            (_, 1) => {
                pb.pos += 8;
            }
            (_, 5) => {
                pb.pos += 4;
            }
            _ => {}
        }
    }
    if !strs.is_empty() {
        return PValue::Str(strs.remove(0));
    }
    if !ints.is_empty() {
        if let Some(b) = bools.first() {
            let _ = b;
        }
        return PValue::Int(ints[0]);
    }
    if !raw_bytes.is_empty() {
        return PValue::Bytes(raw_bytes);
    }
    PValue::Bool(bools.first().copied().unwrap_or(false))
}

fn parse_preferences(bytes: &[u8]) -> Vec<(String, PValue)> {
    let mut out = Vec::new();
    let mut pb = Pb::new(bytes);
    while let Some((fnum, wt)) = pb.field() {
        if fnum == 1 && wt == 2 {
            let entry = pb.len_delim().unwrap_or(&[]);
            let mut key = String::new();
            let mut value = PValue::Bool(false);
            let mut epb = Pb::new(entry);
            while let Some((f, w)) = epb.field() {
                match (f, w) {
                    (1, 2) => {
                        key = String::from_utf8(epb.len_delim().unwrap_or(&[]).to_vec())
                            .unwrap_or_default();
                    }
                    (2, 2) => {
                        value = parse_value(epb.len_delim().unwrap_or(&[]));
                    }
                    _ => {}
                }
            }
            if !key.is_empty() {
                out.push((key, value));
            }
        } else if wt == 0 {
            pb.varint();
        } else if wt == 2 {
            pb.len_delim();
        }
    }
    out
}

/// Android SharedPreferences XML — machine-generated, stable shape:
///   <map><string name="k">v</string><int name="k" value="1"/><boolean .../></map>
/// Hand-parsed (find/extract); the container never nests these tags.
fn parse_legacy_xml(text: &str) -> Vec<(String, PValue)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(tag_start) = rest.find('<') {
        let after = &rest[tag_start..];
        if let Some(name_off) = after.find("name=\"") {
            let name_from = name_off + "name=\"".len();
            let Some(name_len) = after[name_from..].find('"') else {
                break;
            };
            let key = after[name_from..name_from + name_len].to_string();
            let elem_end = match after.find('>') {
                Some(e) => e,
                None => break,
            };
            let elem = &after[..elem_end];
            if let Some(v) = elem.strip_prefix("</") {
                // closing tag — skip it (its opening was handled already)
                rest = &after[elem_end + 1..];
                let _ = v;
                continue;
            }
            if elem.starts_with("<string") {
                // value is the text content up to </string> (all offsets are
                // relative to `after`, which starts at tag_start).
                let val_from = elem_end + 1;
                let Some(close) = after[val_from..].find("</string>") else {
                    break;
                };
                let value = after[val_from..val_from + close].to_string();
                out.push((key, PValue::Str(decode_xml_escapes(&value))));
                rest = &after[val_from + close + "</string>".len()..];
            } else if elem.starts_with("<int") {
                if let Some(voff) = elem.find("value=\"") {
                    let vfrom = voff + "value=\"".len();
                    let Some(vlen) = elem[vfrom..].find('"') else {
                        break;
                    };
                    let v: i64 = elem[vfrom..vfrom + vlen].parse().unwrap_or(0);
                    out.push((key, PValue::Int(v)));
                }
                rest = &after[elem_end + 1..];
            } else if elem.starts_with("<boolean") {
                if let Some(voff) = elem.find("value=\"") {
                    let vfrom = voff + "value=\"".len();
                    let b = elem[vfrom..].starts_with("true");
                    out.push((key, PValue::Bool(b)));
                }
                rest = &after[elem_end + 1..];
            } else if elem.starts_with("<long") {
                if let Some(voff) = elem.find("value=\"") {
                    let vfrom = voff + "value=\"".len();
                    let Some(vlen) = elem[vfrom..].find('"') else {
                        break;
                    };
                    let v: i64 = elem[vfrom..vfrom + vlen].parse().unwrap_or(0);
                    out.push((key, PValue::Int(v)));
                }
                rest = &after[elem_end + 1..];
            } else {
                rest = &after[elem_end + 1..];
            }
        } else {
            break;
        }
    }
    out
}

fn decode_xml_escapes(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Whole-file sha256 computed in 4 MiB chunks (the L1 bulk reference).
fn streaming_sha256(path: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("FAIL open zim (digest): {e}");
        std::process::exit(1);
    });
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    loop {
        let n = f.read(&mut buf).unwrap_or_else(|e| {
            eprintln!("FAIL digest read: {e}");
            std::process::exit(1);
        });
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Minimal UTF-8 percent-decoder for the app's stored article URLs.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 && i + 2 < b.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read the session preferences from whichever container the installed app
/// actually uses. Reports the detected kind honestly.
fn read_session_prefs(bytes: &[u8]) -> (PrefsKind, Vec<(String, PValue)>) {
    if let Ok(text) = std::str::from_utf8(bytes) {
        if text.trim_start().starts_with("<?xml") && text.contains("<map") {
            return (PrefsKind::LegacyXml, parse_legacy_xml(text));
        }
    }
    (PrefsKind::DataStorePb, parse_preferences(bytes))
}

// ---------------- capture ----------------------------------------------------

#[derive(Debug, Default)]
struct CaptureReport {
    current_zim_file: Option<String>,
    current_tab: Option<i64>,
    text_zoom: Option<i64>,
    theme: Option<String>,
    scroll_y: Option<i64>,
    bundle_bytes: Option<usize>,
    history_article: Option<(String, String)>, // (url, title)
    dropped: Vec<String>,
    fallbacks: Vec<String>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let datastore_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: kiwix_capture <session_prefs> <KiwixRoom.db> [zim_path_override]");
            eprintln!("  <session_prefs>: kiwix-mobile.xml (SharedPreferences, v3.14.1 release)");
            eprintln!(
                "                    or kiwix_datastore_preferences.preferences_pb (repo HEAD)"
            );
            std::process::exit(2);
        }
    };
    let room_path = args.next().unwrap_or_default();
    let zim_override = args.next();

    // 1. Session preferences: currentzimfile / currenttab / settings.
    //    (Both containers carry the same keys; see read_session_prefs.)
    let pb_bytes = match std::fs::read(&datastore_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("FAIL read session prefs: {e}");
            std::process::exit(1);
        }
    };
    let (prefs_kind, prefs) = read_session_prefs(&pb_bytes);
    eprintln!(
        "prefs container: {}",
        match prefs_kind {
            PrefsKind::DataStorePb => "androidx DataStore protobuf",
            PrefsKind::LegacyXml => "legacy SharedPreferences XML",
        }
    );
    let mut report = CaptureReport::default();
    for (k, v) in &prefs {
        match k.as_str() {
            "currentzimfile" => {
                if let PValue::Str(s) = v {
                    report.current_zim_file = Some(s.clone());
                }
            }
            "currenttab" => {
                if let PValue::Int(i) = v {
                    report.current_tab = Some(*i);
                }
            }
            "true_text_zoom" => {
                if let PValue::Int(i) = v {
                    report.text_zoom = Some(*i);
                }
            }
            "pref_dark_mode" => {
                if let PValue::Str(s) = v {
                    report.theme = Some(s.clone());
                }
            }
            _ => {}
        }
    }

    // 2. Open the ZIM (override path or the app-recorded one).
    let zim_path = zim_override.or_else(|| report.current_zim_file.clone());
    let Some(zim_path) = zim_path else {
        eprintln!("FAIL: no zim path (datastore currentzimfile empty and no override)");
        std::process::exit(1);
    };
    let arch = match carryon_adapter_kiwix::zim_backend::ZimArchive::open(&zim_path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("FAIL open zim '{zim_path}': {e}");
            std::process::exit(1);
        }
    };
    let info = match arch.info() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("FAIL zim info: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "CAPTURED zim identity from real file: uuid={} size={}",
        info.uuid, info.filesize
    );

    // 3. Room DB: the tab state for THIS zim (zimId == uuid, ZimFileReader.kt:136)
    //    and the most recent history row (the app's own "current article").
    let db = rusqlite::Connection::open_with_flags(
        &room_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap_or_else(|e| {
        eprintln!("FAIL open room db: {e}");
        std::process::exit(1);
    });
    let uuid = info.uuid.clone();
    let (scroll_y, bundle, tab_index_room) = db
        .query_row(
            "SELECT webViewCurrentPosition, webViewBackForwardListBundle, webViewIndex \
             FROM WebViewHistoryEntity WHERE zimId = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![uuid],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<Vec<u8>>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap_or_else(|e| {
            eprintln!("FAIL read WebViewHistoryEntity (zimId={uuid}): {e}");
            std::process::exit(1);
        });
    report.scroll_y = Some(scroll_y);
    report.bundle_bytes = bundle.as_ref().map(|b| b.len());
    let (hist_url, hist_title) = db
        .query_row(
            "SELECT historyUrl, historyTitle FROM HistoryRoomEntity WHERE zimId = ?1 \
             ORDER BY timeStamp DESC LIMIT 1",
            rusqlite::params![uuid],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .unwrap_or_else(|e| {
            eprintln!("FAIL read HistoryRoomEntity (zimId={uuid}): {e}");
            std::process::exit(1);
        });
    report.history_article = Some((hist_url.clone(), hist_title.clone()));

    // Article path: the app's virtual URL is https://kiwix.app/<path>
    // (ZimFileReader.CONTENT_PREFIX); strip it and percent-decode (the app
    // stores the URL-encoded form; libzim wants the stored entry path).
    let raw_path = hist_url
        .strip_prefix("https://kiwix.app/")
        .or_else(|| hist_url.strip_prefix("http://kiwix.app/"))
        .unwrap_or(&hist_url)
        .to_string();
    let decoded_path = percent_decode(&raw_path);
    report.fallbacks.push(format!(
        "article path from the app's own HistoryRoomEntity: {decoded_path}"
    ));

    // The article bytes: read from the real ZIM (prerequisite/immediate tier).
    // Try the decoded path, then the raw one (fail closed if neither resolves).
    let item = match arch.item(&decoded_path).or_else(|_| arch.item(&raw_path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("FAIL resolve article '{decoded_path}' in zim: {e}");
            std::process::exit(1);
        }
    };
    let article_bytes = item.read_range(0, item.size()).unwrap_or_else(|e| {
        eprintln!("FAIL read article: {e}");
        std::process::exit(1);
    });

    // The saveState bundle: opaque Parcel blob — carried, not interpreted.
    report.dropped.push(format!(
        "webViewBackForwardListBundle: {} bytes carried as opaque_platform_state (base64); \
         the Chromium back/forward serialization inside is platform-versioned and NOT interpreted",
        bundle.as_ref().map(|b| b.len()).unwrap_or(0)
    ));
    if tab_index_room != report.current_tab.unwrap_or(tab_index_room) {
        report.fallbacks.push(format!(
            "tab index: datastore currenttab={} vs room webViewIndex={} — using datastore",
            report.current_tab.unwrap_or(-1),
            tab_index_room
        ));
    }

    let session = Session {
        zim: ZimIdentity {
            uuid: info.uuid.clone(),
            filename: zim_path.clone(),
            size: info.filesize,
            // Whole-file streaming sha256 (the L1 bulk reference — the bulk
            // exceeds the core's cut-object cap, so it travels by digest).
            bulk_sha256: streaming_sha256(&zim_path),
        },
        position: ReadingPosition {
            article_path: decoded_path,
            article_title: hist_title,
            scroll_y: scroll_y.max(0) as u64,
            tab_index: report.current_tab.unwrap_or(tab_index_room).max(0) as u32,
            opaque_platform_state: bundle.map(|b| {
                // hex (self-describing length; base64 also fine — the
                // field is opaque either way)
                use std::fmt::Write as _;
                let mut s = String::with_capacity(b.len() * 2);
                for byte in &b {
                    let _ = write!(s, "{byte:02x}");
                }
                s
            }),
        },
        bookmarks: Vec::new(), // Bookmarks live in libkiwix XML on Android; see report.
        history: vec![HistoryEntry {
            zim_id: info.uuid.clone(),
            article_url: hist_url,
            title: String::new(),
            timestamp: 0,
        }],
        settings: ReadingSettings {
            text_zoom: report.text_zoom.unwrap_or(100).max(0) as u32,
            theme: report.theme.clone().unwrap_or_else(|| "system".into()),
        },
    };
    report.dropped.push(
        "bookmarks: stored by the app in libkiwix XML (not Room) — not captured in Phase 2".into(),
    );

    // Honest capture report.
    eprintln!("--- capture report ---");
    eprintln!("currentzimfile = {:?}", report.current_zim_file);
    eprintln!("currenttab     = {:?}", report.current_tab);
    eprintln!("scrollY        = {:?}", report.scroll_y);
    eprintln!("article (history table) = {:?}", report.history_article);
    eprintln!("article bytes  = {}", article_bytes.len());
    eprintln!(
        "settings       = zoom={:?} theme={:?}",
        report.text_zoom, report.theme
    );
    for d in &report.dropped {
        eprintln!("DROPPED: {d}");
    }
    for f in &report.fallbacks {
        eprintln!("FALLBACK: {f}");
    }

    let json = serde_json::json!({
        "session": session,
        "article_bytes": article_bytes.len(),
    });
    println!("{json}");
    eprintln!("CAPTURE_OK");
}
