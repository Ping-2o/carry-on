//! Discovery entries (spec §18.3, stage 4): the ENGINE owns the TXT contract —
//! construction, validation, and the fail-closed receive filter — plus the live
//! mDNS advertise/browse backend used by the desktop shells.
//!
//! Layering (golden rule 4): everything here goes through `carryon-net`; the
//! adapter boundary and the no-core-dep rule are untouched.
//!
//! Platform split (plan stage 4, honest about who responds):
//! - **macOS**: the engine responds — advertise + browse run in Rust
//!   ([`carryon_net::MdnsAdvertiser`] / [`MdnsBrowser`]) via the entries below.
//! - **Android**: the platform NSD responder registers/browses through the app
//!   layer, but its TXT MUST be built through `carryon_discovery_txt_encode`
//!   and every received record validated through `carryon_discovery_txt_validate`
//!   before the app may show or use a peer (the engine decides what a peer is;
//!   a malformed record fails closed — the app drops it, never repairs it).
//!
//! Error mapping: TXT-contract violations are `E_PLATFORM_DISCOVERY` (1105);
//! unparsable JSON input is `ERR_BAD_JSON` (-4).
//!
//! # Safety contract
//! Same as the rest of the ABI: `ffi_guard!` everywhere, out-strings via
//! [`write_out_string`], handles freed with their `*_free`.

use crate::errors::{set_last_error, CARRYON_ERR_BAD_JSON};
use crate::ffi_guard;
use crate::handle::{as_ref, to_handle, CarryonDiscoveryAdvertiser, CarryonDiscoveryBrowser};
use crate::strings::write_out_string;
use carryon_core::carryon_net::{DiscoveredPeer, MdnsAdvertiser, MdnsBrowser, TxtError, TxtRecord};
use std::ffi::c_char;

const BAD_HANDLE: i32 = crate::errors::CARRYON_ERR_BAD_HANDLE;
const PANIC: i32 = crate::errors::CARRYON_ERR_PANIC;

/// `E_PLATFORM_DISCOVERY` — a TXT-contract violation (§18.3 fail-closed filter).
const E_DISCOVERY: i32 = crate::errors::CARRYON_E_PLATFORM_DISCOVERY;

/// Canonical JSON form of a [`TxtRecord`] (the field names the shells see).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TxtFields {
    protocol_major: u16,
    instance_id: String,
    pairing_required: bool,
    device_class: String,
    connection_hint: String,
}

impl TxtFields {
    fn from_record(t: &TxtRecord) -> Self {
        TxtFields {
            protocol_major: t.protocol_major,
            instance_id: t.instance_id.clone(),
            pairing_required: t.pairing_required,
            device_class: match t.device_class {
                carryon_core::carryon_net::DeviceClass::Desktop => "desktop".into(),
                carryon_core::carryon_net::DeviceClass::Mobile => "mobile".into(),
                carryon_core::carryon_net::DeviceClass::Unknown => "unknown".into(),
            },
            connection_hint: t.connection_hint.clone(),
        }
    }

    fn to_record(&self) -> Result<TxtRecord, TxtError> {
        Ok(TxtRecord {
            protocol_major: self.protocol_major,
            instance_id: self.instance_id.clone(),
            pairing_required: self.pairing_required,
            device_class: match self.device_class.as_str() {
                "desktop" => carryon_core::carryon_net::DeviceClass::Desktop,
                "mobile" => carryon_core::carryon_net::DeviceClass::Mobile,
                "unknown" => carryon_core::carryon_net::DeviceClass::Unknown,
                other => return Err(TxtError(format!("unknown device class {other:?}"))),
            },
            connection_hint: self.connection_hint.clone(),
        })
    }
}

/// Validate + canonicalize the §18.3 TXT from FIELD VALUES (what a shell wants
/// to advertise). `fields_json` = [`TxtFields`] JSON; the reply is the canonical
/// `[[key,value],…]` pairs (exactly `pm id pr dc ch`, in that order). The reply
/// is what goes on the wire — construction cannot leak anything else.
///
/// # Safety
/// Both strings valid UTF-8 C strings; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_txt_encode(
    fields_json: *const c_char,
    out_pairs_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(fields) = parse::<TxtFields>(fields_json) else {
            return CARRYON_ERR_BAD_JSON;
        };
        match fields.to_record() {
            Ok(txt) => write_out_string(
                out_pairs_json,
                serde_json::to_string(&txt.to_pairs()).unwrap_or_default(),
            ),
            Err(e) => {
                set_last_error(e.to_string());
                E_DISCOVERY
            }
        }
    })
}

/// Fail-closed validation of a RECEIVED TXT record. `pairs_json` = `[[key,value],…]`
/// pairs exactly as the platform responder handed them over (foreign keys included).
/// On success the reply is `{"txt":{…fields…},"ignored_keys":[…]}`; the shell shows
/// a peer ONLY from the validated fields. Malformed/missing keys →
/// `E_PLATFORM_DISCOVERY` — the caller MUST drop the peer (fail closed, never guess).
///
/// # Safety
/// Both strings valid UTF-8 C strings; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_txt_validate(
    pairs_json: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(pairs) = parse::<Vec<(String, String)>>(pairs_json) else {
            return CARRYON_ERR_BAD_JSON;
        };
        match TxtRecord::from_pairs(&pairs) {
            Ok((txt, ignored)) => {
                let reply = serde_json::json!({
                    "txt": TxtFields::from_record(&txt),
                    "ignored_keys": ignored,
                });
                write_out_string(out_json, reply.to_string())
            }
            Err(e) => {
                set_last_error(e.to_string());
                E_DISCOVERY
            }
        }
    })
}

/// Start engine-side advertising of `_carryon._tcp` (macOS shell path). Null on
/// error; free with `carryon_discovery_advertise_stop`. `txt_pairs_json` must be
/// the canonical pairs from `carryon_discovery_txt_encode` (validated here again —
/// a shell never widens the contract).
///
/// # Safety
/// Strings valid UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_advertise_start(
    instance_name: *const c_char,
    port: u16,
    txt_pairs_json: *const c_char,
) -> *mut CarryonDiscoveryAdvertiser {
    ffi_guard!(std::ptr::null_mut(), {
        let (Some(name), Some(pairs)) = (
            borrow(instance_name),
            parse::<Vec<(String, String)>>(txt_pairs_json),
        ) else {
            set_last_error("instance name or txt pairs null/unparsable");
            return std::ptr::null_mut();
        };
        let txt = match TxtRecord::from_pairs(&pairs) {
            Ok((t, _)) => t,
            Err(e) => {
                set_last_error(e.to_string());
                return std::ptr::null_mut();
            }
        };
        match MdnsAdvertiser::start(&name, port, &txt) {
            Ok(a) => to_handle(a),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// Stop advertising (unregister + daemon shutdown) and free the handle.
///
/// # Safety
/// `adv` from this library (or null), freed once.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_advertise_stop(adv: *mut CarryonDiscoveryAdvertiser) {
    ffi_guard!((), {
        if !adv.is_null() {
            let boxed = Box::from_raw(adv);
            boxed.stop();
        }
    })
}

/// Start engine-side browsing of `_carryon._tcp` with a TTL cache (macOS shell
/// path). Null on error; free with `carryon_discovery_browse_stop`.
///
/// # Safety
/// No arguments.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_browse_start() -> *mut CarryonDiscoveryBrowser {
    ffi_guard!(std::ptr::null_mut(), {
        match MdnsBrowser::start() {
            Ok(b) => to_handle(b),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// Snapshot the current peers (drained, TTL-pruned, §18.3-filtered, `pm`-matched):
/// `{"peers":[{"instance","instance_id","address","txt":{…}}],"rejected":[…],
/// "foreign_keys":[…]}` — `instance` is the DNS-SD fullname (diagnostics/UX),
/// the peer identity is the §18.3 `id` TXT value.
///
/// # Safety
/// `browser` live; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_browse_snapshot(
    browser: *const CarryonDiscoveryBrowser,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(b) = as_ref(browser) else {
            return BAD_HANDLE;
        };
        let peers: Vec<_> = b
            .entries()
            .iter()
            .map(|(fullname, p): &(String, DiscoveredPeer)| {
                serde_json::json!({
                    "instance": fullname,
                    "instance_id": p.instance_id,
                    "address": p.address,
                    "txt": TxtFields::from_record(&p.txt),
                })
            })
            .collect();

        let reply = serde_json::json!({
            "peers": peers,
            "rejected": b.rejections(),
            "foreign_keys": b.foreign_keys().iter().filter(|(_, keys)| !keys.is_empty())
                .map(|(name, keys)| serde_json::json!({"instance": name, "keys": keys}))
                .collect::<Vec<_>>(),
        });
        write_out_string(out_json, reply.to_string())
    })
}

/// Stop browsing and free the handle.
///
/// # Safety
/// `browser` from this library (or null), freed once.
#[no_mangle]
pub unsafe extern "C" fn carryon_discovery_browse_stop(browser: *mut CarryonDiscoveryBrowser) {
    ffi_guard!((), {
        if !browser.is_null() {
            let boxed = Box::from_raw(browser);
            boxed.stop();
        }
    })
}

/// Parse a JSON C string; null on failure (message already set by borrow_str callers).
unsafe fn parse<T: serde::de::DeserializeOwned>(json: *const c_char) -> Option<T> {
    let s = borrow(json)?;
    serde_json::from_str(&s).ok()
}

unsafe fn borrow(s: *const c_char) -> Option<String> {
    if s.is_null() {
        set_last_error("string argument null");
        return None;
    }
    std::ffi::CStr::from_ptr(s)
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read an out-string produced by an entry under test, then free it with
    /// the library's own `carryon_string_free` (one allocator discipline).
    unsafe fn take_out(out: *mut c_char) -> String {
        assert!(!out.is_null(), "out-string must be set on success");
        let s = std::ffi::CStr::from_ptr(out).to_string_lossy().to_string();
        crate::strings::carryon_string_free(out);
        s
    }

    fn encode(fields: serde_json::Value) -> Result<Vec<(String, String)>, (i32, String)> {
        let c = std::ffi::CString::new(fields.to_string()).unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = unsafe { carryon_discovery_txt_encode(c.as_ptr(), &mut out) };
        if rc != crate::errors::CARRYON_OK {
            return Err((rc, last_error_text()));
        }
        let s = unsafe { take_out(out) };
        Ok(serde_json::from_str(&s).expect("pairs json"))
    }

    fn validate(pairs: &[(&str, &str)]) -> Result<(TxtFields, Vec<String>), (i32, String)> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let c = std::ffi::CString::new(serde_json::to_string(&owned).unwrap()).unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = unsafe { carryon_discovery_txt_validate(c.as_ptr(), &mut out) };
        if rc != crate::errors::CARRYON_OK {
            return Err((rc, last_error_text()));
        }
        let s = unsafe { take_out(out) };
        let v: serde_json::Value = serde_json::from_str(&s).expect("validate json");
        Ok((
            serde_json::from_value(v["txt"].clone()).expect("txt fields"),
            serde_json::from_value(v["ignored_keys"].clone()).expect("ignored keys"),
        ))
    }

    fn last_error_text() -> String {
        let mut len: usize = 0;
        crate::errors::copy_last_error(std::ptr::null_mut(), &mut len);
        let mut buf = vec![0u8; len];
        let mut n = len;
        crate::errors::copy_last_error(buf.as_mut_ptr(), &mut n);
        buf.truncate(n.saturating_sub(1));
        String::from_utf8_lossy(&buf).to_string()
    }

    #[test]
    fn encode_produces_canonical_pairs() {
        let pairs = encode(serde_json::json!({
            "protocol_major": 1, "instance_id": "ab12cd34",
            "pairing_required": true, "device_class": "mobile",
            "connection_hint": "54321",
        }))
        .expect("encode");
        let keys: Vec<_> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["pm", "id", "pr", "dc", "ch"]);
        assert_eq!(pairs[0].1, "1");
        assert_eq!(pairs[2].1, "1");
    }

    #[test]
    fn encode_refuses_bad_device_class() {
        let (rc, msg) = encode(serde_json::json!({
            "protocol_major": 1, "instance_id": "x",
            "pairing_required": false, "device_class": "iPhone 14",
            "connection_hint": "",
        }))
        .expect_err("bad device class must refuse");
        assert_eq!(rc, E_DISCOVERY);
        assert!(msg.contains("device class"), "message: {msg}");
    }

    #[test]
    fn validate_roundtrip_and_ignores_foreign_keys() {
        let mut pairs = encode(serde_json::json!({
            "protocol_major": 1, "instance_id": "ab12cd34",
            "pairing_required": false, "device_class": "desktop",
            "connection_hint": "none",
        }))
        .expect("encode");
        pairs.push(("vendor_key".into(), "opaque".into()));
        let pairs_ref: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let (txt, ignored) = validate(&pairs_ref).expect("validate");
        assert_eq!(txt.protocol_major, 1);
        assert!(!txt.pairing_required);
        assert_eq!(txt.device_class, "desktop");
        assert_eq!(txt.connection_hint, "none");
        assert_eq!(ignored, vec!["vendor_key"]);
    }

    #[test]
    fn validate_fails_closed_on_malformed() {
        let (rc, msg) = validate(&[("pm", "one")]).expect_err("malformed must refuse");
        assert_eq!(rc, E_DISCOVERY);
        assert!(msg.contains("pm"), "message: {msg}");
        let (rc, _) = validate(&[]).expect_err("empty TXT must refuse");
        assert_eq!(rc, E_DISCOVERY);
        // unparsable JSON -> ERR_BAD_JSON
        let c = std::ffi::CString::new("not json").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = unsafe { carryon_discovery_txt_validate(c.as_ptr(), &mut out) };
        assert_eq!(rc, CARRYON_ERR_BAD_JSON);
    }
}
