//! Green gate: Rust acts as the C caller (unsafe extern), proving the full
//! lifecycle + panic-safety WITHOUT any shell.
//!
//! LOCAL EVIDENCE ONLY (spec §2/§30): loopback over 127.0.0.1 with real TLS 1.3.
//! Not physical cross-device evidence; no platform is "supported".

use carryon_ffi::*;
use std::ffi::{c_char, CStr, CString};
use std::ptr;
use std::thread;
use tempfile::tempdir;

// Re-declare the extern functions as a C caller would (they live in the cdylib; the
// rlib lets us link them directly in-test).
use carryon_ffi::abi_core::*;
use carryon_ffi::abi_net::*;
use carryon_ffi::abi_transfer::*;
use carryon_ffi::handle::{CarryonCore, CarryonIdentity, CarryonListener, CarryonTrust};

/// Read an out-string and free it.
unsafe fn take_string(p: *mut c_char) -> String {
    assert!(!p.is_null(), "null out-string");
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    carryon_ffi::strings::carryon_string_free(p);
    s
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[test]
fn ffi_full_lifecycle_local() {
    unsafe {
        let dir = tempdir().unwrap();
        let data = cstr(dir.path().to_str().unwrap());
        let core = carryon_core_open(data.as_ptr());
        assert!(!core.is_null());

        // Register the graph adapter by id + JSON params (the "import" entry point).
        let mut info_json: *mut c_char = ptr::null_mut();
        let rc = carryon_register_adapter(
            core,
            cstr("org.carryon.graph").as_ptr(),
            cstr(r#"{"sample":true}"#).as_ptr(),
            &mut info_json,
        );
        assert_eq!(rc, 0, "register_adapter");
        let _ = take_string(info_json);

        // Create session.
        let mut sid: *mut c_char = ptr::null_mut();
        let rc = carryon_create_session(
            core,
            cstr(r#"{"adapter_id":"org.carryon.graph","title":"g","privacy":"public","authority_mode":"read_only_replica"}"#).as_ptr(),
            &mut sid,
        );
        assert_eq!(rc, 0, "create_session");
        let session_id = take_string(sid);

        // Seal a cut.
        let mut cut_json: *mut c_char = ptr::null_mut();
        let rc = carryon_create_cut(core, cstr(&session_id).as_ptr(), &mut cut_json);
        assert_eq!(rc, 0, "create_cut");
        let cut = take_string(cut_json);

        // Execute shortest_path.
        let mut res_json: *mut c_char = ptr::null_mut();
        let rc = carryon_execute_action(
            core,
            cstr(&cut).as_ptr(),
            cstr(r#"{"class":"graph.shortest_path","params":{"start":0,"end":4}}"#).as_ptr(),
            &mut res_json,
        );
        assert_eq!(rc, 0, "execute_action");
        let res = take_string(res_json);
        assert!(res.contains("\"agreed\":true"), "oracle agreed: {res}");

        // Export + verify evidence.
        let mut bundle_json: *mut c_char = ptr::null_mut();
        let rc = carryon_export_evidence(core, cstr(&session_id).as_ptr(), &mut bundle_json);
        assert_eq!(rc, 0, "export_evidence");
        let bundle = take_string(bundle_json);
        let bundle_path = dir.path().join("ffi-evidence.json");
        std::fs::write(&bundle_path, &bundle).unwrap();
        let mut report_json: *mut c_char = ptr::null_mut();
        let rc = carryon_verify_evidence(
            core,
            cstr(bundle_path.to_str().unwrap()).as_ptr(),
            &mut report_json,
        );
        assert_eq!(rc, 0, "verify_evidence");
        let report = take_string(report_json);
        assert!(report.contains("\"ok\":true"), "evidence ok: {report}");

        carryon_core_free(core);
    }
}

#[cfg(feature = "test-panic")]
#[test]
fn ffi_panic_returns_error_not_abort() {
    extern "C" {
        fn carryon_debug_panic() -> i32;
    }
    unsafe {
        let rc = carryon_debug_panic();
        assert_eq!(rc, -1, "panic must become CARRYON_ERR_PANIC, not abort");
        // Process did not abort: a subsequent normal call still works.
        let dir = tempdir().unwrap();
        let core = carryon_core_open(cstr(dir.path().to_str().unwrap()).as_ptr());
        assert!(!core.is_null());
        carryon_core_free(core);
    }
}

#[test]
fn ffi_identity_persist_roundtrip() {
    unsafe {
        let id = carryon_identity_generate(cstr("alice").as_ptr());
        assert!(!id.is_null());

        // Export cert + key DER via the *len query protocol.
        let read_der =
            |f: unsafe extern "C" fn(*const CarryonIdentity, *mut u8, *mut usize) -> i32| {
                let mut len: usize = 0;
                let rc = f(id, ptr::null_mut(), &mut len);
                assert_eq!(rc, -6, "expected BUFFER_TOO_SMALL for size query");
                let mut buf = vec![0u8; len];
                let rc = f(id, buf.as_mut_ptr(), &mut len);
                assert_eq!(rc, 0);
                buf.truncate(len);
                buf
            };
        let cert = read_der(carryon_identity_cert_der);
        let key = read_der(carryon_identity_key_der);

        let mut pin1: *mut c_char = ptr::null_mut();
        carryon_identity_pin_hex(id, &mut pin1);
        let pin1 = take_string(pin1);

        // Rehydrate from DER → stable pin across "launches".
        let id2 = carryon_identity_from_der(
            cstr("alice").as_ptr(),
            cert.as_ptr(),
            cert.len(),
            key.as_ptr(),
            key.len(),
        );
        assert!(!id2.is_null());
        let mut pin2: *mut c_char = ptr::null_mut();
        carryon_identity_pin_hex(id2, &mut pin2);
        let pin2 = take_string(pin2);
        assert_eq!(pin1, pin2, "pin stable across from_der");

        carryon_identity_free(id);
        carryon_identity_free(id2);
    }
}

/// Drive a full handoff over TLS via the FFI: source serves, destination imports.
#[test]
fn ffi_handoff_loopback_import() {
    unsafe {
        // Source core with a sealed graph cut.
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();
        let src = carryon_core_open(cstr(src_dir.path().to_str().unwrap()).as_ptr());
        let dst = carryon_core_open(cstr(dst_dir.path().to_str().unwrap()).as_ptr());
        assert!(!src.is_null() && !dst.is_null());

        let mut info: *mut c_char = ptr::null_mut();
        carryon_register_adapter(
            src,
            cstr("org.carryon.graph").as_ptr(),
            cstr(r#"{"sample":true}"#).as_ptr(),
            &mut info,
        );
        let _ = take_string(info);
        let mut sid: *mut c_char = ptr::null_mut();
        carryon_create_session(
            src,
            cstr(r#"{"adapter_id":"org.carryon.graph","title":"g","privacy":"public","authority_mode":"read_only_replica"}"#).as_ptr(),
            &mut sid,
        );
        let session_id = take_string(sid);
        let mut cut_json: *mut c_char = ptr::null_mut();
        carryon_create_cut(src, cstr(&session_id).as_ptr(), &mut cut_json);
        let cut: serde_json::Value = serde_json::from_str(&take_string(cut_json)).unwrap();
        let cut_num = cut["number"].as_u64().unwrap();

        // Identities + pairing via FFI.
        let sid_id = carryon_identity_generate(cstr("source").as_ptr());
        let did_id = carryon_identity_generate(cstr("dest").as_ptr());
        let mut pair_json: *mut c_char = ptr::null_mut();
        carryon_pair_devices(sid_id, did_id, cstr("t").as_ptr(), &mut pair_json);
        let pair: serde_json::Value = serde_json::from_str(&take_string(pair_json)).unwrap();
        let src_trust = carryon_trust_from_json(cstr(&pair["local_trust"].to_string()).as_ptr());
        let dst_trust = carryon_trust_from_json(cstr(&pair["remote_trust"].to_string()).as_ptr());
        assert!(!src_trust.is_null() && !dst_trust.is_null());

        // Listener + address.
        let listener = carryon_listener_bind(cstr("127.0.0.1:0").as_ptr());
        assert!(!listener.is_null());
        let mut addr_s: *mut c_char = ptr::null_mut();
        carryon_listener_addr(listener, &mut addr_s);
        let addr = take_string(addr_s);

        // Source serves on a thread. Pointers are sent across via usize (opaque).
        let src_u = src as usize;
        let sid_id_u = sid_id as usize;
        let src_trust_u = src_trust as usize;
        let listener_u = listener as usize;
        let server = thread::spawn(move || {
            let sess = carryon_session_accept(
                listener_u as *const CarryonListener,
                sid_id_u as *const CarryonIdentity,
                src_trust_u as *const CarryonTrust,
            );
            assert!(!sess.is_null());
            carryon_session_server_negotiate(sess, cstr("[]").as_ptr(), &mut (ptr::null_mut()));
            let rc = carryon_serve_cut(src_u as *mut CarryonCore, sess);
            assert_eq!(rc, 0, "serve_cut");
            carryon_session_free(sess);
        });

        // Destination connects + imports.
        let sess = carryon_session_connect(cstr(&addr).as_ptr(), did_id, dst_trust);
        assert!(!sess.is_null());
        let mut neg: *mut c_char = ptr::null_mut();
        carryon_session_client_negotiate(sess, cstr("[]").as_ptr(), &mut neg);
        let _ = take_string(neg);
        let mut out: *mut c_char = ptr::null_mut();
        let rc = carryon_import_cut(dst, sess, cstr(&session_id).as_ptr(), cut_num, &mut out);
        assert_eq!(rc, 0, "import_cut");
        let result = take_string(out);
        assert!(
            result.contains("\"completed\":true"),
            "import completed: {result}"
        );

        server.join().unwrap();
        carryon_session_free(sess);
        carryon_listener_free(listener);
        carryon_identity_free(sid_id);
        carryon_identity_free(did_id);
        carryon_trust_free(src_trust);
        carryon_trust_free(dst_trust);
        carryon_core_free(src);
        carryon_core_free(dst);
    }
}

#[test]
fn ffi_budget_rejection() {
    unsafe {
        let (src_dir, dst_dir) = (tempdir().unwrap(), tempdir().unwrap());
        let src = carryon_core_open(cstr(src_dir.path().to_str().unwrap()).as_ptr());
        let dst = carryon_core_open(cstr(dst_dir.path().to_str().unwrap()).as_ptr());

        let mut info: *mut c_char = ptr::null_mut();
        carryon_register_adapter(
            src,
            cstr("org.carryon.graph").as_ptr(),
            cstr(r#"{"sample":true}"#).as_ptr(),
            &mut info,
        );
        let _ = take_string(info);
        let mut sid: *mut c_char = ptr::null_mut();
        carryon_create_session(src, cstr(r#"{"adapter_id":"org.carryon.graph","title":"g","privacy":"public","authority_mode":"read_only_replica"}"#).as_ptr(), &mut sid);
        let session_id = take_string(sid);
        let mut cut_json: *mut c_char = ptr::null_mut();
        carryon_create_cut(src, cstr(&session_id).as_ptr(), &mut cut_json);
        let cut: serde_json::Value = serde_json::from_str(&take_string(cut_json)).unwrap();
        let cut_num = cut["number"].as_u64().unwrap();

        // Tiny net budget → import must be refused.
        let budget = serde_json::json!({
            "total_net_bytes": 1u64, "avg_net_rate": 1u64, "burst_bytes": 1u64,
            "total_cpu_ms": 60000u64, "cpu_duty": 1.0, "peak_prep_mem": 536870912u64,
            "storage_quota": 18446744073709551615u64, "battery_thermal": "Unrestricted",
            "elapsed_opportunity_ms": 60000u64, "max_nonpreemptible_ms": 5000u64
        });
        let rc = carryon_core_set_budget(dst, cstr(&budget.to_string()).as_ptr());
        assert_eq!(rc, 0, "set_budget");

        let sid_id = carryon_identity_generate(cstr("source").as_ptr());
        let did_id = carryon_identity_generate(cstr("dest").as_ptr());
        let mut pair_json: *mut c_char = ptr::null_mut();
        carryon_pair_devices(sid_id, did_id, cstr("t").as_ptr(), &mut pair_json);
        let pair: serde_json::Value = serde_json::from_str(&take_string(pair_json)).unwrap();
        let src_trust = carryon_trust_from_json(cstr(&pair["local_trust"].to_string()).as_ptr());
        let dst_trust = carryon_trust_from_json(cstr(&pair["remote_trust"].to_string()).as_ptr());

        let listener = carryon_listener_bind(cstr("127.0.0.1:0").as_ptr());
        let mut addr_s: *mut c_char = ptr::null_mut();
        carryon_listener_addr(listener, &mut addr_s);
        let addr = take_string(addr_s);

        let (src_u, sid_id_u, src_trust_u, listener_u) = (
            src as usize,
            sid_id as usize,
            src_trust as usize,
            listener as usize,
        );
        let server = thread::spawn(move || {
            let sess = carryon_session_accept(
                listener_u as *const CarryonListener,
                sid_id_u as *const CarryonIdentity,
                src_trust_u as *const CarryonTrust,
            );
            if !sess.is_null() {
                carryon_session_server_negotiate(sess, cstr("[]").as_ptr(), &mut (ptr::null_mut()));
                let _ = carryon_serve_cut(src_u as *mut CarryonCore, sess);
                carryon_session_free(sess);
            }
        });

        let sess = carryon_session_connect(cstr(&addr).as_ptr(), did_id, dst_trust);
        let mut neg: *mut c_char = ptr::null_mut();
        carryon_session_client_negotiate(sess, cstr("[]").as_ptr(), &mut neg);
        let _ = take_string(neg);
        let mut out: *mut c_char = ptr::null_mut();
        let rc = carryon_import_cut(dst, sess, cstr(&session_id).as_ptr(), cut_num, &mut out);
        assert_eq!(rc, 500, "expected CARRYON_E_BUDGET_NETWORK (500)");

        // Budget refused the import after the manifest exchange; the source is still
        // blocked in serve_cut's recv. Close the client session FIRST so that recv
        // errors and the server thread exits, then join (avoids a deadlock).
        carryon_session_free(sess);
        let _ = server.join();
        carryon_listener_free(listener);
        carryon_identity_free(sid_id);
        carryon_identity_free(did_id);
        carryon_trust_free(src_trust);
        carryon_trust_free(dst_trust);
        carryon_core_free(src);
        carryon_core_free(dst);
    }
}

/// Suspend an import mid-flight via the FFI, then resume it to completion.
#[test]
fn ffi_suspend_then_resume() {
    unsafe {
        let (src_dir, dst_dir) = (tempdir().unwrap(), tempdir().unwrap());
        let src = carryon_core_open(cstr(src_dir.path().to_str().unwrap()).as_ptr());
        let dst = carryon_core_open(cstr(dst_dir.path().to_str().unwrap()).as_ptr());
        // Tiny chunks so suspend lands at a clean boundary.
        assert_eq!(carryon_core_set_chunk_size(dst, 4096), 0);

        let mut info: *mut c_char = ptr::null_mut();
        carryon_register_adapter(
            src,
            cstr("org.carryon.graph").as_ptr(),
            cstr(r#"{"sample":true}"#).as_ptr(),
            &mut info,
        );
        let _ = take_string(info);
        let mut sid: *mut c_char = ptr::null_mut();
        carryon_create_session(src, cstr(r#"{"adapter_id":"org.carryon.graph","title":"g","privacy":"public","authority_mode":"read_only_replica"}"#).as_ptr(), &mut sid);
        let session_id = take_string(sid);
        let mut cut_json: *mut c_char = ptr::null_mut();
        carryon_create_cut(src, cstr(&session_id).as_ptr(), &mut cut_json);
        let cut: serde_json::Value = serde_json::from_str(&take_string(cut_json)).unwrap();
        let cut_num = cut["number"].as_u64().unwrap();

        let sid_id = carryon_identity_generate(cstr("source").as_ptr());
        let did_id = carryon_identity_generate(cstr("dest").as_ptr());
        let mut pair_json: *mut c_char = ptr::null_mut();
        carryon_pair_devices(sid_id, did_id, cstr("t").as_ptr(), &mut pair_json);
        let pair: serde_json::Value = serde_json::from_str(&take_string(pair_json)).unwrap();
        let src_trust = carryon_trust_from_json(cstr(&pair["local_trust"].to_string()).as_ptr());
        let dst_trust = carryon_trust_from_json(cstr(&pair["remote_trust"].to_string()).as_ptr());

        let listener = carryon_listener_bind(cstr("127.0.0.1:0").as_ptr());
        let mut addr_s: *mut c_char = ptr::null_mut();
        carryon_listener_addr(listener, &mut addr_s);
        let addr = take_string(addr_s);

        // Source serves BOTH the suspended attempt and the resume (two connections).
        let (src_u, sid_id_u, src_trust_u, listener_u) = (
            src as usize,
            sid_id as usize,
            src_trust as usize,
            listener as usize,
        );
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let sess = carryon_session_accept(
                    listener_u as *const CarryonListener,
                    sid_id_u as *const CarryonIdentity,
                    src_trust_u as *const CarryonTrust,
                );
                if sess.is_null() {
                    break;
                }
                carryon_session_server_negotiate(sess, cstr("[]").as_ptr(), &mut (ptr::null_mut()));
                let _ = carryon_serve_cut(src_u as *mut CarryonCore, sess);
                carryon_session_free(sess);
            }
        });

        // Suspend before the first chunk so the import suspends immediately.
        carryon_core_request_suspend(dst);
        let sess1 = carryon_session_connect(cstr(&addr).as_ptr(), did_id, dst_trust);
        let mut neg: *mut c_char = ptr::null_mut();
        carryon_session_client_negotiate(sess1, cstr("[]").as_ptr(), &mut neg);
        let _ = take_string(neg);
        let mut out1: *mut c_char = ptr::null_mut();
        let rc = carryon_import_cut(dst, sess1, cstr(&session_id).as_ptr(), cut_num, &mut out1);
        assert_eq!(rc, 0, "import call ok");
        let r1: serde_json::Value = serde_json::from_str(&take_string(out1)).unwrap();
        assert_eq!(
            r1["suspended"],
            serde_json::json!(true),
            "must suspend: {r1}"
        );
        let token = r1["resume_token"].to_string();
        carryon_session_free(sess1); // close first connection

        // Resume on a fresh connection → completes.
        let sess2 = carryon_session_connect(cstr(&addr).as_ptr(), did_id, dst_trust);
        let mut neg2: *mut c_char = ptr::null_mut();
        carryon_session_client_negotiate(sess2, cstr("[]").as_ptr(), &mut neg2);
        let _ = take_string(neg2);
        let mut out2: *mut c_char = ptr::null_mut();
        let rc = carryon_resume_import(dst, sess2, cstr(&token).as_ptr(), &mut out2);
        assert_eq!(rc, 0, "resume ok");
        let r2: serde_json::Value = serde_json::from_str(&take_string(out2)).unwrap();
        assert_eq!(
            r2["completed"],
            serde_json::json!(true),
            "must complete: {r2}"
        );

        carryon_session_free(sess2);
        server.join().unwrap();
        carryon_listener_free(listener);
        carryon_identity_free(sid_id);
        carryon_identity_free(did_id);
        carryon_trust_free(src_trust);
        carryon_trust_free(dst_trust);
        carryon_core_free(src);
        carryon_core_free(dst);
    }
}

#[test]
fn ffi_abi_version_matches() {
    unsafe {
        let mut maj = 0u32;
        let mut min = 0u32;
        carryon_abi_version(&mut maj, &mut min);
        assert_eq!(maj, carryon_ffi::CARRYON_ABI_MAJOR);
        assert_eq!(min, carryon_ffi::CARRYON_ABI_MINOR);
    }
}

/// L4 authority transfer through the C ABI end to end (loopback). Source is a
/// single-writer editor; after serving the cut it relinquishes authority; the
/// destination accepts and becomes the owner. Asserts `carryon_may_mutate` flips.
///
/// LOCAL EVIDENCE ONLY (spec §2/§30).
#[test]
fn ffi_authority_transfer_moves_ownership() {
    unsafe {
        let (src_dir, dst_dir) = (tempdir().unwrap(), tempdir().unwrap());
        let src = carryon_core_open(cstr(src_dir.path().to_str().unwrap()).as_ptr());
        let dst = carryon_core_open(cstr(dst_dir.path().to_str().unwrap()).as_ptr());
        assert!(!src.is_null() && !dst.is_null());

        // Source editor (single-writer), sealed cut.
        let mut info: *mut c_char = ptr::null_mut();
        carryon_register_adapter(
            src,
            cstr("org.carryon.editor").as_ptr(),
            cstr(r#"{"sample":true}"#).as_ptr(),
            &mut info,
        );
        let _ = take_string(info);
        let mut sid: *mut c_char = ptr::null_mut();
        carryon_create_session(
            src,
            cstr(r#"{"adapter_id":"org.carryon.editor","title":"e","privacy":"personal","authority_mode":"single_writer"}"#).as_ptr(),
            &mut sid,
        );
        let session_id = take_string(sid);
        let mut cut_json: *mut c_char = ptr::null_mut();
        carryon_create_cut(src, cstr(&session_id).as_ptr(), &mut cut_json);
        let cut: serde_json::Value = serde_json::from_str(&take_string(cut_json)).unwrap();
        let cut_num = cut["number"].as_u64().unwrap();

        // Destination editor with the SAME content, so the proposal's content-hash
        // binding matches the imported state.
        let mut dinfo: *mut c_char = ptr::null_mut();
        carryon_register_adapter(
            dst,
            cstr("org.carryon.editor").as_ptr(),
            cstr(r#"{"session":"mirror","text":"cooperative draft v1"}"#).as_ptr(),
            &mut dinfo,
        );
        let _ = take_string(dinfo);

        // Pairing + listener.
        let sid_id = carryon_identity_generate(cstr("source").as_ptr());
        let did_id = carryon_identity_generate(cstr("dest").as_ptr());
        let mut pair_json: *mut c_char = ptr::null_mut();
        carryon_pair_devices(sid_id, did_id, cstr("t").as_ptr(), &mut pair_json);
        let pair: serde_json::Value = serde_json::from_str(&take_string(pair_json)).unwrap();
        let src_trust = carryon_trust_from_json(cstr(&pair["local_trust"].to_string()).as_ptr());
        let dst_trust = carryon_trust_from_json(cstr(&pair["remote_trust"].to_string()).as_ptr());
        let listener = carryon_listener_bind(cstr("127.0.0.1:0").as_ptr());
        let mut addr_s: *mut c_char = ptr::null_mut();
        carryon_listener_addr(listener, &mut addr_s);
        let addr = take_string(addr_s);

        // Mirror id the destination will own.
        let mut mirror_s: *mut c_char = ptr::null_mut();
        let rc = carryon_mirror_session_id(cstr(&session_id).as_ptr(), &mut mirror_s);
        assert_eq!(rc, 0);
        let mirror_id = take_string(mirror_s);

        // Source thread: serve the cut, then relinquish authority.
        let src_u = src as usize;
        let sid_id_u = sid_id as usize;
        let src_trust_u = src_trust as usize;
        let listener_u = listener as usize;
        let session_id_src = session_id.clone();
        let server = thread::spawn(move || {
            let sess = carryon_session_accept(
                listener_u as *const CarryonListener,
                sid_id_u as *const CarryonIdentity,
                src_trust_u as *const CarryonTrust,
            );
            carryon_session_server_negotiate(sess, cstr("[]").as_ptr(), &mut (ptr::null_mut()));
            let rc = carryon_serve_cut(src_u as *mut CarryonCore, sess);
            assert_eq!(rc, 0, "serve_cut");
            // Source still owns before transfer.
            let mut owns = false;
            carryon_may_mutate(
                src_u as *const CarryonCore,
                cstr(&session_id_src).as_ptr(),
                &mut owns,
            );
            assert!(owns, "source owns before transfer");
            let mut receipt: *mut c_char = ptr::null_mut();
            let rc = carryon_serve_authority_transfer(
                src_u as *mut CarryonCore,
                sess,
                cstr(&session_id_src).as_ptr(),
                cut_num,
                &mut receipt,
            );
            assert_eq!(rc, 0, "serve_authority_transfer");
            let _ = take_string(receipt);
            // Source dropped to read-only.
            let mut still = true;
            carryon_may_mutate(
                src_u as *const CarryonCore,
                cstr(&session_id_src).as_ptr(),
                &mut still,
            );
            assert!(!still, "source read-only after relinquishment");
            carryon_session_free(sess);
        });

        // Destination: import then accept authority.
        let sess = carryon_session_connect(cstr(&addr).as_ptr(), did_id, dst_trust);
        let mut neg: *mut c_char = ptr::null_mut();
        carryon_session_client_negotiate(sess, cstr("[]").as_ptr(), &mut neg);
        let _ = take_string(neg);
        let mut out: *mut c_char = ptr::null_mut();
        let rc = carryon_import_cut(dst, sess, cstr(&session_id).as_ptr(), cut_num, &mut out);
        assert_eq!(rc, 0, "import_cut");
        let _ = take_string(out);

        // Before: mirror read-only.
        let mut pre = true;
        carryon_may_mutate(dst, cstr(&mirror_id).as_ptr(), &mut pre);
        assert!(!pre, "mirror read-only before transfer");

        let mut receipt: *mut c_char = ptr::null_mut();
        let rc = carryon_request_authority_transfer(
            dst,
            sess,
            cstr(&mirror_id).as_ptr(),
            cstr("org.carryon.editor").as_ptr(),
            &mut receipt,
        );
        assert_eq!(rc, 0, "request_authority_transfer");
        let set = take_string(receipt);
        assert!(set.contains("\"new_epoch\":1"), "epoch advanced: {set}");

        // After: destination owns authority.
        let mut post = false;
        carryon_may_mutate(dst, cstr(&mirror_id).as_ptr(), &mut post);
        assert!(post, "destination owns authority after transfer");

        server.join().unwrap();
        carryon_session_free(sess);
        carryon_listener_free(listener);
        carryon_identity_free(sid_id);
        carryon_identity_free(did_id);
        carryon_trust_free(src_trust);
        carryon_trust_free(dst_trust);
        carryon_core_free(src);
        carryon_core_free(dst);
    }
}
