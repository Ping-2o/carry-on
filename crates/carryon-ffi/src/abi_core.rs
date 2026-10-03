//! Core lifecycle + adapter/session/cut/action/evidence entry points.

use crate::errors::{record, set_last_error, CARRYON_ERR_BAD_JSON, CARRYON_OK};
use crate::ffi_guard;
use crate::handle::{as_mut, as_ref, free, to_handle, CarryonCore};
use crate::strings::{borrow_str, write_out_string};
use carryon_core::model::{ActionRequest, AuthorityMode, Budget, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use serde_json::Value;
use std::ffi::c_char;

const NULL: i32 = crate::errors::CARRYON_ERR_NULL_ARG;
const BAD_HANDLE: i32 = crate::errors::CARRYON_ERR_BAD_HANDLE;
const PANIC: i32 = crate::errors::CARRYON_ERR_PANIC;

/// Open (or create) an engine rooted at `data_dir`. Returns a `*mut CarryonCore`
/// (null on error; call `carryon_last_error`). Free with `carryon_core_free`.
///
/// # Safety
/// `data_dir` is a valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_open(data_dir: *const c_char) -> *mut CarryonCore {
    ffi_guard!(std::ptr::null_mut(), {
        let dir = match borrow_str(data_dir) {
            Ok(s) => s,
            Err(_) => {
                set_last_error("data_dir null or not UTF-8");
                return std::ptr::null_mut();
            }
        };
        match Core::open(std::path::Path::new(dir)) {
            Ok(c) => to_handle(c),
            Err(e) => {
                record(&e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Free a core handle. Null-tolerant.
///
/// # Safety
/// `core` is a handle from `carryon_core_open` (or null), freed at most once.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_free(core: *mut CarryonCore) {
    ffi_guard!((), { free(core) })
}

/// Recovery report (§19.3) as JSON into `*out_json`.
///
/// # Safety
/// `core` live; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_recovery_report_json(
    core: *mut CarryonCore,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_ref(core as *const CarryonCore) else {
            return BAD_HANDLE;
        };
        let r = core.recovery_report();
        let json = serde_json::json!({
            "interrupted_transfers": r.interrupted_transfers,
            "hidden_incomplete_objects": r.hidden_incomplete_objects,
            "ambiguous_sessions": r.ambiguous_sessions,
            "journal_truncated": r.journal_truncated,
            "clean": r.is_clean(),
        });
        write_out_string(out_json, json.to_string())
    })
}

/// Set the transfer chunk size in bytes (0 = reset to default). §18.6.
///
/// # Safety
/// `core` live.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_set_chunk_size(core: *mut CarryonCore, bytes: u64) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let arg = if bytes == 0 { None } else { Some(bytes) };
        match core.set_chunk_size(arg) {
            Ok(()) => CARRYON_OK,
            Err(e) => record(&e),
        }
    })
}

/// Set the active budget from JSON (§6.8).
///
/// # Safety
/// `core` live; `budget_json` valid UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_set_budget(
    core: *mut CarryonCore,
    budget_json: *const c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let s = match borrow_str(budget_json) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let budget: Budget = match serde_json::from_str(s) {
            Ok(b) => b,
            Err(_) => {
                set_last_error("bad budget JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        core.set_budget(budget);
        CARRYON_OK
    })
}

/// Set foreground (true) / background (false). Background shrinks the budget (§20.2).
///
/// # Safety
/// `core` live.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_set_foreground(core: *mut CarryonCore, fg: bool) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        core.set_foreground(fg);
        CARRYON_OK
    })
}

/// Request cooperative suspension of an in-flight transfer (§11.2). Thread-safe:
/// this is the only entry safe to call while an import runs on another thread.
///
/// # Safety
/// `core` live for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn carryon_core_request_suspend(core: *mut CarryonCore) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_ref(core as *const CarryonCore) else {
            return BAD_HANDLE;
        };
        core.request_suspend();
        CARRYON_OK
    })
}

/// Register a compiled-in adapter by id + JSON params (the "import a program" entry
/// point, §3.8). Writes the `AdapterInfo` JSON into `*out_info_json`.
///
/// # Safety
/// `core` live; `adapter_id`/`params_json` valid UTF-8 C strings; `out_info_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_register_adapter(
    core: *mut CarryonCore,
    adapter_id: *const c_char,
    params_json: *const c_char,
    out_info_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let id = match borrow_str(adapter_id) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let params_str = match borrow_str(params_json) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let params: Value = match serde_json::from_str(params_str) {
            Ok(v) => v,
            Err(_) => {
                set_last_error("bad adapter params JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let adapter = match crate::factory::build_adapter(id, &params) {
            Ok(a) => a,
            Err(e) => return record(&e),
        };
        match core.register_adapter(adapter) {
            Ok(info) => match serde_json::to_string(&info) {
                Ok(j) => write_out_string(out_info_json, j),
                Err(_) => CARRYON_ERR_BAD_JSON,
            },
            Err(e) => record(&e),
        }
    })
}

/// Create a session from a JSON request `{adapter_id,title,privacy,authority_mode}`.
/// Writes the session id (UUID hex) into `*out_session_id`.
///
/// # Safety
/// `core` live; `create_req_json` valid; `out_session_id` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_create_session(
    core: *mut CarryonCore,
    create_req_json: *const c_char,
    out_session_id: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let s = match borrow_str(create_req_json) {
            Ok(s) => s,
            Err(c) => return c,
        };
        let v: Value = match serde_json::from_str(s) {
            Ok(v) => v,
            Err(_) => {
                set_last_error("bad create-session JSON");
                return CARRYON_ERR_BAD_JSON;
            }
        };
        let req = CreateSessionReq {
            adapter_id: v
                .get("adapter_id")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            title: v
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("session")
                .to_string(),
            privacy: parse_sensitivity(v.get("privacy").and_then(|x| x.as_str())),
            authority_mode: parse_authority(v.get("authority_mode").and_then(|x| x.as_str())),
        };
        match core.create_session(req) {
            Ok(id) => write_out_string(out_session_id, id.to_string()),
            Err(e) => record(&e),
        }
    })
}

/// Seal a cut for a session id. Writes `{session,number}` JSON into `*out_cut_json`.
///
/// # Safety
/// `core` live; `session_id` valid; `out_cut_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_create_cut(
    core: *mut CarryonCore,
    session_id: *const c_char,
    out_cut_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let sid = match borrow_str(session_id).ok().and_then(parse_session) {
            Some(s) => s,
            None => {
                set_last_error("bad session id");
                return NULL;
            }
        };
        match core.create_cut(sid) {
            Ok(cut) => write_out_string(
                out_cut_json,
                serde_json::json!({ "session": cut.session.to_string(), "number": cut.number })
                    .to_string(),
            ),
            Err(e) => record(&e),
        }
    })
}

/// List which of `classes_json` (a JSON array of strings) are runnable at a cut
/// (`cut_json` = `{session,number}`). Writes a JSON array into `*out_json`.
///
/// # Safety
/// `core` live; inputs valid; `out_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_list_available_actions(
    core: *mut CarryonCore,
    cut_json: *const c_char,
    classes_json: *const c_char,
    out_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_ref(core as *const CarryonCore) else {
            return BAD_HANDLE;
        };
        let cut = match borrow_str(cut_json).ok().and_then(parse_cut) {
            Some(c) => c,
            None => return CARRYON_ERR_BAD_JSON,
        };
        let classes: Vec<String> = match borrow_str(classes_json)
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
        {
            Some(c) => c,
            None => return CARRYON_ERR_BAD_JSON,
        };
        let avail = core.list_available_actions(cut, &classes);
        let arr: Vec<_> = avail
            .into_iter()
            .map(|a| serde_json::json!({ "class": a.class, "ready": a.ready }))
            .collect();
        write_out_string(out_json, Value::from(arr).to_string())
    })
}

/// Execute an action at a cut. `action_req_json` = `{class,params}`. Writes the
/// `ActionResult` (output + oracle) JSON into `*out_result_json`.
///
/// # Safety
/// `core` live; inputs valid; `out_result_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_execute_action(
    core: *mut CarryonCore,
    cut_json: *const c_char,
    action_req_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let cut = match borrow_str(cut_json).ok().and_then(parse_cut) {
            Some(c) => c,
            None => return CARRYON_ERR_BAD_JSON,
        };
        let v: Value = match borrow_str(action_req_json)
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
        {
            Some(v) => v,
            None => return CARRYON_ERR_BAD_JSON,
        };
        let req = ActionRequest {
            class: v
                .get("class")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
        };
        match core.execute_action(cut, req) {
            Ok(res) => write_out_string(out_result_json, action_result_json(&res)),
            Err(e) => record(&e),
        }
    })
}

/// Execute an action at a cut, driven by an explicitly named registered adapter.
/// Needed for source-off continuation of an imported mirror session (its recorded
/// adapter id is the placeholder `imported`).
///
/// # Safety
/// `core` live; `cut_json`/`action_req_json`/`adapter_id` valid; `out_result_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_execute_action_as(
    core: *mut CarryonCore,
    cut_json: *const c_char,
    action_req_json: *const c_char,
    adapter_id: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let cut = match borrow_str(cut_json).ok().and_then(parse_cut) {
            Some(c) => c,
            None => return CARRYON_ERR_BAD_JSON,
        };
        let aid = match borrow_str(adapter_id) {
            Ok(s) => s.to_string(),
            Err(c) => return c,
        };
        let v: Value = match borrow_str(action_req_json)
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
        {
            Some(v) => v,
            None => return CARRYON_ERR_BAD_JSON,
        };
        let req = ActionRequest {
            class: v
                .get("class")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
        };
        match core.execute_action_as(cut, req, &aid) {
            Ok(res) => write_out_string(out_result_json, action_result_json(&res)),
            Err(e) => record(&e),
        }
    })
}

fn action_result_json(res: &carryon_core::model::ActionResult) -> String {
    serde_json::json!({
        "output": res.output,
        "output_hash": res.output_hash.to_hex(),
        "oracle": {
            "checked": res.oracle.checked,
            "agreed": res.oracle.agreed,
            "detail": res.oracle.detail,
        }
    })
    .to_string()
}

/// Whether an object with `content_hash` (64 hex) is present + verified locally.
///
/// # Safety
/// `core` live; `content_hash` valid; `out_present` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_has_object_hex(
    core: *mut CarryonCore,
    content_hash: *const c_char,
    out_present: *mut bool,
) -> i32 {
    ffi_guard!(PANIC, {
        if out_present.is_null() {
            return NULL;
        }
        let Some(core) = as_ref(core as *const CarryonCore) else {
            return BAD_HANDLE;
        };
        let h = match borrow_str(content_hash) {
            Ok(s) => s,
            Err(c) => return c,
        };
        *out_present = core.has_object_hex(h);
        CARRYON_OK
    })
}

/// Export an evidence bundle for a session as JSON (§23.3).
///
/// # Safety
/// `core` live; `session_id` valid; `out_bundle_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_export_evidence(
    core: *mut CarryonCore,
    session_id: *const c_char,
    out_bundle_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let sid = match borrow_str(session_id).ok().and_then(parse_session) {
            Some(s) => s,
            None => return NULL,
        };
        match core.export_evidence(sid) {
            Ok(b) => match serde_json::to_string(&b) {
                Ok(j) => write_out_string(out_bundle_json, j),
                Err(_) => CARRYON_ERR_BAD_JSON,
            },
            Err(e) => record(&e),
        }
    })
}

/// Assemble an evidence bundle, merging caller-measured continuation metrics
/// (`extra_metrics_json`, a JSON object) into the bundle's `metrics` section. The
/// driver measures timings/bytes/transferred-vs-optional around the handoff and
/// passes them here so they are covered by the bundle's section hashes.
///
/// # Safety
/// `core` live; `session_id`/`extra_metrics_json` valid; `out_bundle_json` writable.
#[no_mangle]
pub unsafe extern "C" fn carryon_export_evidence_with(
    core: *mut CarryonCore,
    session_id: *const c_char,
    extra_metrics_json: *const c_char,
    out_bundle_json: *mut *mut c_char,
) -> i32 {
    ffi_guard!(PANIC, {
        let Some(core) = as_mut(core) else {
            return BAD_HANDLE;
        };
        let sid = match borrow_str(session_id).ok().and_then(parse_session) {
            Some(s) => s,
            None => return NULL,
        };
        let extra: Value = borrow_str(extra_metrics_json)
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        match core.export_evidence_with(sid, extra) {
            Ok(b) => match serde_json::to_string(&b) {
                Ok(j) => write_out_string(out_bundle_json, j),
                Err(_) => CARRYON_ERR_BAD_JSON,
            },
            Err(e) => record(&e),
        }
    })
}

// --- small parse helpers (shared across abi modules via pub(crate)) ---

pub(crate) fn parse_session(s: &str) -> Option<carryon_core::ids::SessionId> {
    uuid::Uuid::parse_str(s)
        .ok()
        .map(carryon_core::ids::SessionId)
}

pub(crate) fn parse_cut(s: &str) -> Option<carryon_core::ids::CutId> {
    let v: Value = serde_json::from_str(s).ok()?;
    let session = parse_session(v.get("session")?.as_str()?)?;
    let number = v.get("number")?.as_u64()?;
    Some(carryon_core::ids::CutId { session, number })
}

fn parse_sensitivity(s: Option<&str>) -> Sensitivity {
    match s {
        Some("personal") => Sensitivity::Personal,
        Some("confidential") => Sensitivity::Confidential,
        Some("secret") => Sensitivity::Secret,
        Some("prohibited") => Sensitivity::Prohibited,
        _ => Sensitivity::Public,
    }
}

fn parse_authority(s: Option<&str>) -> AuthorityMode {
    match s {
        Some("single_writer") => AuthorityMode::SingleWriter,
        _ => AuthorityMode::ReadOnlyReplica,
    }
}
