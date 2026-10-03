//! Handoff entry points: serve (source), import + resume (destination), and trust
//! mutation (pair/revoke) by pin (spec §18.6/§11.2).

use crate::abi_core::parse_session;
use crate::errors::{record, set_last_error, CARRYON_ERR_BAD_JSON, CARRYON_OK};
use crate::ffi_guard;
use crate::handle::{as_mut, as_ref};
use crate::handle::{CarryonCore, CarryonSession, CarryonTrust};
use crate::strings::{borrow_str, write_out_string};
use carryon_core::carryon_net::Pin;
use carryon_core::{ImportOutcome, ImportResume};
use std::ffi::c_char;

const NULL: i32 = crate::errors::CARRYON_ERR_NULL_ARG;
const BAD_HANDLE: i32 = crate::errors::CARRYON_ERR_BAD_HANDLE;
const PANIC: i32 = crate::errors::CARRYON_ERR_PANIC;

/// Serve a sealed cut's bytes to a connected destination until it reports
/// `ImportComplete` (source side). Read-only; never mutates source state.
///
/// # Safety
/// `core`/`session` live for the call.
#[no_mangle]
pub unsafe extern "C" fn carryon_serve_cut(
    core: *mut CarryonCore,
    session: *mut CarryonSession,
) -> i32 {
    ffi_guard!(PANIC, {
        let (Some(core), Some(sess)) = (as_mut(core), as_mut(session)) else {
            return BAD_HANDLE;
        };
        match core.serve_cut(sess) {
            Ok(()) => CARRYON_OK,
            Err(e) => record(&e),
        }
    })
}

/// Import a remote cut (destination side). Writes a result JSON into `*out_result_json`:
/// `{ "completed": true, "mirror_cut": N }` or
/// `{ "suspended": true, "resume_token": {…} }`.
///
/// # Safety
/// `core`/`session` live; `remote_session` valid; `out_result_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_import_cut(
    core: *mut CarryonCore,
    session: *mut CarryonSession,
    remote_session: *const c_char,
    remote_cut: u64,
    out_result_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let (Some(core), Some(sess)) = (as_mut(core), as_mut(session)) else {
            return BAD_HANDLE;
        };
        let rs = match borrow_str(remote_session) {
            Ok(s) => s,
            Err(c) => return c,
        };
        match core.import_cut(sess, rs, remote_cut) {
            Ok(outcome) => write_outcome(out_result_json, &outcome),
            Err(e) => record(&e),
        }
    })
}

/// Resume a suspended import from a token JSON (as emitted by `carryon_import_cut`).
///
/// # Safety
/// `core`/`session` live; `resume_token_json` valid; `out_result_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_resume_import(
    core: *mut CarryonCore,
    session: *mut CarryonSession,
    resume_token_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let (Some(core), Some(sess)) = (as_mut(core), as_mut(session)) else {
            return BAD_HANDLE;
        };
        let token: ImportResume = match borrow_str(resume_token_json)
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
        {
            Some(t) => t,
            None => {
                set_last_error("bad resume token JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        match core.resume_import(sess, &token) {
            Ok(outcome) => write_outcome(out_result_json, &outcome),
            Err(e) => record(&e),
        }
    })
}

/// Pair a peer into a trust store by pin hex (§18.4).
///
/// # Safety
/// `trust` live; `name`/`pin_hex`/`utc` valid.
#[no_mangle]
pub unsafe extern "C" fn carryon_trust_pair(
    trust: *mut CarryonTrust,
    name: *const c_char,
    pin_hex: *const c_char,
    utc: *const c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(t) = as_mut(trust) else {
            return BAD_HANDLE;
        };
        let name = borrow_str(name).unwrap_or("peer");
        let pin = match borrow_str(pin_hex).ok().and_then(Pin::from_hex) {
            Some(p) => p,
            None => return NULL,
        };
        let utc = borrow_str(utc).unwrap_or("paired");
        t.pair(name, pin, utc);
        CARRYON_OK
    })
}

/// Whether a pin is currently trusted.
///
/// # Safety
/// `trust` live; `pin_hex` valid; `out` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_trust_is_trusted(
    trust: *const CarryonTrust,
    pin_hex: *const c_char,
    out: *mut bool,
) -> i32 {
    ffi_guard!(PANIC, {
        if out.is_null() {
            return NULL;
        }
        let Some(t) = as_ref(trust) else {
            return BAD_HANDLE;
        };
        let pin = match borrow_str(pin_hex).ok().and_then(Pin::from_hex) {
            Some(p) => p,
            None => return NULL,
        };
        *out = t.is_trusted(&pin);
        CARRYON_OK
    })
}

/// Revoke a peer by pin (§18.4.9); blocks reconnect.
///
/// # Safety
/// `trust` live; `pin_hex` valid; `out` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_trust_revoke(
    trust: *mut CarryonTrust,
    pin_hex: *const c_char,
    out: *mut bool,
) -> i32 {
    ffi_guard!(PANIC, {
        if out.is_null() {
            return NULL;
        }
        let Some(t) = as_mut(trust) else {
            return BAD_HANDLE;
        };
        let pin = match borrow_str(pin_hex).ok().and_then(Pin::from_hex) {
            Some(p) => p,
            None => return NULL,
        };
        *out = t.revoke(&pin);
        CARRYON_OK
    })
}

/// Verify an evidence bundle on disk (parse + hash-check only; never executes bundle
/// content, EVD-005). Writes `{ok,message}` JSON into `*out_report_json`.
///
/// # Safety
/// `core` live; `bundle_path` valid; `out_report_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_verify_evidence(
    core: *const CarryonCore,
    bundle_path: *const c_char,
    out_report_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_ref(core) else {
            return BAD_HANDLE;
        };
        let path = match borrow_str(bundle_path) {
            Ok(s) => s,
            Err(c) => return c,
        };
        match core.verify_evidence(std::path::Path::new(path)) {
            Ok(r) => write_out_string(
                out_report_json,
                serde_json::json!({ "ok": r.ok, "message": r.message }).to_string(),
            ),
            Err(e) => record(&e),
        }
    })
}

/// Read a content-addressed object's full bytes from the local store by 64-hex
/// digest, using the `*len`-query byte-copy protocol (pass `buf=null` to learn the
/// length, then call again with a buffer of that size). Lets a shell pull a carried
/// object (e.g. navigation state) out through the ABI.
///
/// # Safety
/// `core` live; `content_hash` valid; `buf` has `*len` bytes or null to query; `len` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_read_object(
    core: *const CarryonCore,
    content_hash: *const c_char,
    buf: *mut u8,
    len: *mut usize,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_ref(core) else {
            return BAD_HANDLE;
        };
        if len.is_null() {
            return NULL;
        }
        let hash = match borrow_str(content_hash) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let bytes = match core.read_object_hex(hash) {
            Ok(b) => b,
            Err(e) => return record(&e),
        };
        let cap = *len;
        *len = bytes.len();
        if buf.is_null() || cap < bytes.len() {
            return crate::errors::CARRYON_ERR_BUFFER_TOO_SMALL;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
        CARRYON_OK
    })
}

/// Read the measured wire-byte counters of a transport session (bytes actually
/// sent/received on this session — length prefix + body, not estimates).
///
/// # Safety
/// `session` live; `out_sent`/`out_recv` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_session_bytes(
    session: *const CarryonSession,
    out_sent: *mut u64,
    out_recv: *mut u64,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(s) = as_ref(session) else {
            return BAD_HANDLE;
        };
        if out_sent.is_null() || out_recv.is_null() {
            return NULL;
        }
        *out_sent = s.bytes_sent();
        *out_recv = s.bytes_recv();
        CARRYON_OK
    })
}

fn write_outcome(out: *mut *mut c_char, outcome: &ImportOutcome) -> i32 {
    let json = match outcome {
        ImportOutcome::Completed(n) => {
            serde_json::json!({ "completed": true, "mirror_cut": n })
        }
        ImportOutcome::Suspended(token) => {
            serde_json::json!({ "suspended": true, "resume_token": token })
        }
    };
    write_out_string(out, json.to_string())
}

// --- Authority transfer (L4, §21.2) ----------------------------------------

/// Source side: offer + relinquish single-writer authority for `cut_number` of
/// `session_id` over `session`. Writes the receipt set JSON into `*out_receipt_json`.
/// On success this device drops to a read-only replica.
///
/// # Safety
/// `core`/`session` live; `session_id` valid; `out_receipt_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_serve_authority_transfer(
    core: *mut CarryonCore,
    session: *mut CarryonSession,
    session_id: *const c_char,
    cut_number: u64,
    out_receipt_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let (Some(core), Some(sess)) = (as_mut(core), as_mut(session)) else {
            return BAD_HANDLE;
        };
        let Some(sid) = borrow_str(session_id).ok().and_then(parse_session) else {
            set_last_error("bad session id");
            return CARRYON_ERR_BAD_JSON;
        };
        match core.serve_authority_transfer(sess, sid, cut_number) {
            Ok(set) => write_out_string(
                out_receipt_json,
                serde_json::to_string(&set).unwrap_or_else(|_| "{}".into()),
            ),
            Err(e) => record(&e),
        }
    })
}

/// Destination side: accept an offered authority transfer onto the local mirror
/// `mirror_session_id`, continued by the registered `adapter_id`. Writes the receipt
/// set JSON into `*out_receipt_json`. On success this device becomes the owner.
///
/// # Safety
/// `core`/`session` live; `mirror_session_id`/`adapter_id` valid; out writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_request_authority_transfer(
    core: *mut CarryonCore,
    session: *mut CarryonSession,
    mirror_session_id: *const c_char,
    adapter_id: *const c_char,
    out_receipt_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let (Some(core), Some(sess)) = (as_mut(core), as_mut(session)) else {
            return BAD_HANDLE;
        };
        let Some(sid) = borrow_str(mirror_session_id).ok().and_then(parse_session) else {
            set_last_error("bad mirror session id");
            return CARRYON_ERR_BAD_JSON;
        };
        let aid = match borrow_str(adapter_id) {
            Ok(s) => s,
            Err(c) => return c,
        };
        match core.request_authority_transfer(sess, sid, aid) {
            Ok(set) => write_out_string(
                out_receipt_json,
                serde_json::to_string(&set).unwrap_or_else(|_| "{}".into()),
            ),
            Err(e) => record(&e),
        }
    })
}

/// Manually recover an ambiguous session (§21.3): open a fresh owned epoch and
/// record a conflict-risk event. Writes the new epoch into `*out_epoch`.
///
/// # Safety
/// `core` live; `session_id` valid; `out_epoch` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_recover_authority(
    core: *mut CarryonCore,
    session_id: *const c_char,
    out_epoch: *mut u64,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        if out_epoch.is_null() {
            return NULL;
        }
        let Some(sid) = borrow_str(session_id).ok().and_then(parse_session) else {
            set_last_error("bad session id");
            return CARRYON_ERR_BAD_JSON;
        };
        match core.recover_authority(sid) {
            Ok(epoch) => {
                *out_epoch = epoch.0;
                CARRYON_OK
            }
            Err(e) => record(&e),
        }
    })
}

/// Query whether this device may authoritatively mutate `session_id` right now
/// (AUTH-004/005). Writes the boolean into `*out`.
///
/// # Safety
/// `core` live; `session_id` valid; `out` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_may_mutate(
    core: *const CarryonCore,
    session_id: *const c_char,
    out: *mut bool,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_ref(core) else {
            return BAD_HANDLE;
        };
        if out.is_null() {
            return NULL;
        }
        let Some(sid) = borrow_str(session_id).ok().and_then(parse_session) else {
            set_last_error("bad session id");
            return CARRYON_ERR_BAD_JSON;
        };
        *out = core.may_mutate(sid);
        CARRYON_OK
    })
}

/// Compute the deterministic local mirror session id for a remote session string.
/// A destination passes this to `carryon_request_authority_transfer`. Writes the
/// mirror id (UUID string) into `*out_mirror_id`.
///
/// # Safety
/// `remote_session` valid; `out_mirror_id` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_mirror_session_id(
    remote_session: *const c_char,
    out_mirror_id: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let rs = match borrow_str(remote_session) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let mirror = carryon_core::Core::mirror_session_id(rs);
        write_out_string(out_mirror_id, mirror.to_string())
    })
}
