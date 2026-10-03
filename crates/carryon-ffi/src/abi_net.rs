//! Transport entry points: device identity (incl. persistence), trust store,
//! pairing, listener/session, and negotiation (spec §18).

use crate::errors::{set_last_error, CARRYON_ERR_BAD_JSON, CARRYON_OK};
use crate::ffi_guard;
use crate::handle::{as_mut, as_ref, free, to_handle};
use crate::handle::{CarryonIdentity, CarryonListener, CarryonSession, CarryonTrust};
use crate::strings::{borrow_str, write_out_string};
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session, TrustStore};
use std::ffi::c_char;
use std::net::TcpListener;
use std::sync::Arc;

const NULL: i32 = crate::errors::CARRYON_ERR_NULL_ARG;
const BAD_HANDLE: i32 = crate::errors::CARRYON_ERR_BAD_HANDLE;
const PANIC: i32 = crate::errors::CARRYON_ERR_PANIC;
const BUF_SMALL: i32 = crate::errors::CARRYON_ERR_BUFFER_TOO_SMALL;

/// Generate a fresh device identity. Null on error; free with `carryon_identity_free`.
///
/// # Safety
/// `name` valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn carryon_identity_generate(name: *const c_char) -> *mut CarryonIdentity {
    ffi_guard!(std::ptr::null_mut(), {
        let n = match borrow_str(name) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };
        match DeviceIdentity::generate(n) {
            Ok(id) => to_handle(id),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// Rehydrate a device identity from persisted DER bytes (stable pin across launches,
/// §18.4). The key DER is SECRET — the shell must read it from secure storage.
///
/// # Safety
/// `name` valid; `cert`/`key` point to `cert_len`/`key_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn carryon_identity_from_der(
    name: *const c_char,
    cert: *const u8,
    cert_len: usize,
    key: *const u8,
    key_len: usize,
) -> *mut CarryonIdentity {
    ffi_guard!(std::ptr::null_mut(), {
        let n = match borrow_str(name) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };
        if cert.is_null() || key.is_null() {
            set_last_error("cert/key pointer null");
            return std::ptr::null_mut();
        }
        let cert_der = std::slice::from_raw_parts(cert, cert_len).to_vec();
        let key_der = std::slice::from_raw_parts(key, key_len).to_vec();
        match DeviceIdentity::from_der(n, cert_der, key_der) {
            Ok(id) => to_handle(id),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// Copy the certificate DER (public) into a caller buffer. `*len` out = required.
///
/// # Safety
/// `id` live; `buf` has `*len` bytes or null to query; `len` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_identity_cert_der(
    id: *const CarryonIdentity,
    buf: *mut u8,
    len: *mut usize,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(id) = as_ref(id) else {
            return BAD_HANDLE;
        };
        copy_bytes(&id.cert_der, buf, len)
    })
}

/// Copy the private key DER into a caller buffer. **SECRET** — route straight to
/// platform secure storage (Keychain/Keystore/DPAPI, §8.7); never log or bundle it.
///
/// # Safety
/// As `carryon_identity_cert_der`.
#[no_mangle]
pub unsafe extern "C" fn carryon_identity_key_der(
    id: *const CarryonIdentity,
    buf: *mut u8,
    len: *mut usize,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(id) = as_ref(id) else {
            return BAD_HANDLE;
        };
        copy_bytes(id.export_key_der(), buf, len)
    })
}

/// The device pin (SHA-256 of the cert DER) as 64-hex into `*out_hex`.
///
/// # Safety
/// `id` live; `out_hex` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_identity_pin_hex(
    id: *const CarryonIdentity,
    out_hex: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(id) = as_ref(id) else {
            return BAD_HANDLE;
        };
        write_out_string(out_hex, id.pin().to_hex())
    })
}

/// Free an identity handle. Null-tolerant.
///
/// # Safety
/// `id` from this library (or null), freed once.
#[no_mangle]
pub unsafe extern "C" fn carryon_identity_free(id: *mut CarryonIdentity) {
    ffi_guard!((), { free(id) })
}

/// Pair two in-process identities (loopback/testing convenience). Writes
/// `{local_trust, remote_trust}` JSON (each a serialized TrustStore) into `*out_json`.
///
/// # Safety
/// `a`/`b` live; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_pair_devices(
    a: *const CarryonIdentity,
    b: *const CarryonIdentity,
    utc: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let (Some(a), Some(b)) = (as_ref(a), as_ref(b)) else {
            return BAD_HANDLE;
        };
        let u = borrow_str(utc).unwrap_or("paired");
        match pair_devices(a, b, u) {
            Ok((lt, rt)) => {
                let json = serde_json::json!({ "local_trust": lt, "remote_trust": rt });
                write_out_string(out_json, json.to_string())
            }
            Err(e) => {
                set_last_error(e.to_string());
                PANIC
            }
        }
    })
}

/// Build a trust store from JSON (e.g. one half of `carryon_pair_devices`). Null on
/// error; free with `carryon_trust_free`.
///
/// # Safety
/// `json` valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn carryon_trust_from_json(json: *const c_char) -> *mut CarryonTrust {
    ffi_guard!(std::ptr::null_mut(), {
        let s = match borrow_str(json) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };
        match serde_json::from_str::<TrustStore>(s) {
            Ok(t) => to_handle(t),
            Err(_) => {
                set_last_error("bad trust-store JSON");
                std::ptr::null_mut()
            }
        }
    })
}

/// Serialize a trust store to JSON for persistence.
///
/// # Safety
/// `trust` live; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_trust_to_json(
    trust: *const CarryonTrust,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(t) = as_ref(trust) else {
            return BAD_HANDLE;
        };
        match serde_json::to_string(t) {
            Ok(j) => write_out_string(out_json, j),
            Err(_) => CARRYON_ERR_BAD_JSON,
        }
    })
}

/// Free a trust handle. Null-tolerant.
///
/// # Safety
/// `trust` from this library (or null), freed once.
#[no_mangle]
pub unsafe extern "C" fn carryon_trust_free(trust: *mut CarryonTrust) {
    ffi_guard!((), { free(trust) })
}

/// Bind a TCP listener on `addr` (e.g. `"127.0.0.1:0"`). Null on error; free with
/// `carryon_listener_free`. Use `carryon_listener_addr` to read the bound address.
///
/// # Safety
/// `addr` valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn carryon_listener_bind(addr: *const c_char) -> *mut CarryonListener {
    ffi_guard!(std::ptr::null_mut(), {
        let a = match borrow_str(addr) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };
        match TcpListener::bind(a) {
            Ok(l) => to_handle(l),
            Err(e) => {
                set_last_error(format!("bind: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}

/// The listener's bound socket address (`host:port`) into `*out_addr`.
///
/// # Safety
/// `listener` live; `out_addr` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_listener_addr(
    listener: *const CarryonListener,
    out_addr: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(l) = as_ref(listener) else {
            return BAD_HANDLE;
        };
        match l.local_addr() {
            Ok(a) => write_out_string(out_addr, a.to_string()),
            Err(e) => {
                set_last_error(format!("local_addr: {e}"));
                PANIC
            }
        }
    })
}

/// Free a listener handle. Null-tolerant.
///
/// # Safety
/// `listener` from this library (or null), freed once.
#[no_mangle]
pub unsafe extern "C" fn carryon_listener_free(listener: *mut CarryonListener) {
    ffi_guard!((), { free(listener) })
}

/// Connect as the client to `addr`, pinning the server against `trust` (TLS 1.3,
/// §18.5). Null on error; free with `carryon_session_free`.
///
/// # Safety
/// `addr` valid; `id`/`trust` live.
#[no_mangle]
pub unsafe extern "C" fn carryon_session_connect(
    addr: *const c_char,
    id: *const CarryonIdentity,
    trust: *const CarryonTrust,
) -> *mut CarryonSession {
    ffi_guard!(std::ptr::null_mut(), {
        let a = match borrow_str(addr) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };
        let (Some(id), Some(trust)) = (as_ref(id), as_ref(trust)) else {
            set_last_error("identity or trust handle null");
            return std::ptr::null_mut();
        };
        match Session::connect(a, id, Arc::new(trust.clone())) {
            Ok(s) => to_handle(s),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// Accept one connection from `listener` as the server, pinning the client.
///
/// # Safety
/// `listener`/`id`/`trust` live.
#[no_mangle]
pub unsafe extern "C" fn carryon_session_accept(
    listener: *const CarryonListener,
    id: *const CarryonIdentity,
    trust: *const CarryonTrust,
) -> *mut CarryonSession {
    ffi_guard!(std::ptr::null_mut(), {
        let (Some(l), Some(id), Some(trust)) = (as_ref(listener), as_ref(id), as_ref(trust)) else {
            set_last_error("listener/identity/trust handle null");
            return std::ptr::null_mut();
        };
        match Session::accept(l, id, Arc::new(trust.clone())) {
            Ok(s) => to_handle(s),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// Client negotiation: send Hello(features), expect Welcome. `features_json` is a
/// JSON array of strings; the peer's features are written into `*out_features_json`.
///
/// # Safety
/// `session` live; inputs valid; `out_features_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_session_client_negotiate(
    session: *mut CarryonSession,
    features_json: *const c_char,
    out_features_json: *mut *mut c_char,
) -> i32 {
    negotiate(session, features_json, out_features_json, true)
}

/// Server negotiation: expect Hello, reply Welcome(features).
///
/// # Safety
/// As `carryon_session_client_negotiate`.
#[no_mangle]
pub unsafe extern "C" fn carryon_session_server_negotiate(
    session: *mut CarryonSession,
    features_json: *const c_char,
    out_features_json: *mut *mut c_char,
) -> i32 {
    negotiate(session, features_json, out_features_json, false)
}

/// Free a session handle. Null-tolerant.
///
/// # Safety
/// `session` from this library (or null), freed once.
#[no_mangle]
pub unsafe extern "C" fn carryon_session_free(session: *mut CarryonSession) {
    ffi_guard!((), { free(session) })
}

unsafe fn negotiate(
    session: *mut CarryonSession,
    features_json: *const c_char,
    out_features_json: *mut *mut c_char,
    client: bool,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(s) = as_mut(session) else {
            return BAD_HANDLE;
        };
        let features: Vec<String> = borrow_str(features_json)
            .ok()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let res = if client {
            s.client_negotiate(features)
        } else {
            s.server_negotiate(features)
        };
        match res {
            Ok(theirs) => write_out_string(
                out_features_json,
                serde_json::to_string(&theirs).unwrap_or_else(|_| "[]".into()),
            ),
            Err(e) => {
                set_last_error(e.to_string());
                PANIC
            }
        }
    })
}

/// Copy a byte slice into a caller buffer with the `*len`-query protocol.
unsafe fn copy_bytes(src: &[u8], buf: *mut u8, len: *mut usize) -> i32 {
    if len.is_null() {
        return NULL;
    }
    let cap = *len;
    *len = src.len();
    if buf.is_null() || cap < src.len() {
        return BUF_SMALL;
    }
    std::ptr::copy_nonoverlapping(src.as_ptr(), buf, src.len());
    CARRYON_OK
}
