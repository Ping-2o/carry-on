/*
 * carryon_jni.c — app-local JNI shim over the REAL Carry-On C ABI.
 *
 * One JNI function per ABI entry (crates/carryon-ffi/include/carryon.h, ABI 1.1,
 * append-only). The crate's FFI surface is untouched (plan.md hard rule); this
 * shim is pure translation:
 *
 *   - Handles:      engine pointers stay opaque in Kotlin as jlong. Null handle ->
 *                     jlong 0; the engine validates handles itself (BAD_HANDLE).
 *   - Out-strings:  the ABI hands back a Rust-allocated NUL-terminated UTF-8
 *                     buffer (`char**`). The shim copies the bytes into a
 *                     jbyteArray and frees the original with carryon_string_free
 *                     — ownership never crosses to Kotlin, and Kotlin decodes
 *                     UTF-8 (not modified UTF-8, so any engine JSON round-trips).
 *   - In-strings:   GetStringUTFChars (modified UTF-8). Identical to UTF-8 for
 *                     the ASCII JSON this app sends; supplementary characters
 *                     would be surrogate-paired (documented limitation).
 *   - Out-buffers:  cert/key DER, last_error, read_object use the ABI's *len
 *                     query protocol verbatim: buf=NULL queries the required
 *                     size, CARRYON_ERR_BUFFER_TOO_SMALL reports it via *len.
 *                     The shim pre-checks every caller-declared cap against
 *                     GetArrayLength (len_precheck) — the engine trusts cap,
 *                     so a mismatched (buf, len) pair must die HERE, not as a
 *                     native out-of-bounds write.
 *   - Errors:       every fallible call returns the ABI i32 (0 = CARRYON_OK;
 *                     negative = FFI-local; positive = family_base + ordinal).
 *                     The shim NEVER throws; Kotlin maps codes to names
 *                     (com.carryon.app.ffi.Err, mirror of errors.rs). The
 *                     jbyteArray out-string wrappers cannot RETURN the i32
 *                     (null on error), so they record it thread-locally
 *                     (out_string_result + the pthread-key rc, read back via
 *                     lastOutRc on the SAME thread — device-run bug
 *                     2026-11-07: the code used to be lost as UNKNOWN).
 *
 * Secrets (§22.3, EVD-006): identityCertDer/identityKeyDer exist to mirror the
 * ABI and for future secure-storage persistence. They are public-but-uncalled
 * on the Kotlin side (JNI binding is by name, so visibility cannot restrict it
 * without an `internal` name-mangle); the app-level API exposes ONLY the
 * SHA-256 pin hex — the key DER is never logged, displayed, or exported
 * (see CarryonCore.kt).
 *
 * No entry point here runs foreign code: registerAdapter selects a compiled-in
 * adapter by id + JSON params (data, never behavior — spec §3.8).
 */

#include <jni.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "carryon.h"

/* ---- handle + string helpers ------------------------------------------- */

static void *h2p(jlong h) { return (void *)(intptr_t)h; }
static jlong p2h(const void *p) { return (jlong)(intptr_t)p; }

/** Borrow a Kotlin String for one call. NULL Kotlin string -> pass the NULL
 *  through so the engine reports CARRYON_ERR_NULL_ARG (no silent ""). */
static const char *borrow_utf(JNIEnv *env, jstring js) {
    if (!js) return NULL;
    return (*env)->GetStringUTFChars(env, js, NULL);
}

static void release_utf(JNIEnv *env, jstring js, const char *c) {
    if (js && c) (*env)->ReleaseStringUTFChars(env, js, c);
}

/** Copy an ABI out-string (Rust-owned, NUL-terminated UTF-8) into a fresh
 *  jbyteArray and free the original. NULL input -> NULL (caller saw rc != OK). */
static jbyteArray out_bytes(JNIEnv *env, char *s) {
    if (!s) return NULL;
    jsize n = (jsize)strlen(s);
    jbyteArray arr = (*env)->NewByteArray(env, n);
    if (arr && n > 0)
        (*env)->SetByteArrayRegion(env, arr, 0, n, (const jbyte *)s);
    carryon_string_free(s);
    return arr;
}

/** Thread-local rc of the last out-string engine call (device-run bug
 *  2026-11-07). The JNI signature for out-string wrappers returns
 *  jbyteArray — NULL on error — which CANNOT carry the engine's i32 error
 *  code; the Kotlin fallback used to fabricate Err.UNKNOWN (INT_MIN), so a
 *  typed outcome like the §6.8 E_BUDGET_NETWORK refusal (500) could not be
 *  matched and the graph+budget run wedged until the watchdog.
 *
 *  Implemented with a pthread key, NOT a __thread variable — belt and braces
 *  after the 2026-11-07 device crash (SIGSEGV stack overflow with the bionic
 *  TLSDESC resolver in the loop; a shared shim under dlopen with
 *  ANDROID_STL=none has no reason to ride the TLSDESC path at all).
 *  Same-thread discipline as the engine's own thread-local
 *  carryon_last_error: set by every out-string wrapper below, read by Kotlin
 *  IMMEDIATELY after the failing call on the SAME thread (requireOut does). */
static pthread_key_t g_out_rc_key;
static pthread_once_t g_out_rc_once = PTHREAD_ONCE_INIT;

static void out_rc_key_init(void) {
    pthread_key_create(&g_out_rc_key, NULL);
}

static void set_out_rc(jint rc) {
    pthread_once(&g_out_rc_once, out_rc_key_init);
    /* rc == 0 stores NULL (cleared) — fine: the value is only read on the
     * error path, where the rc is never 0. */
    pthread_setspecific(g_out_rc_key, (void *)(intptr_t)rc);
}

static jint get_out_rc(void) {
    pthread_once(&g_out_rc_once, out_rc_key_init);
    void *v = pthread_getspecific(g_out_rc_key);
    return v ? (jint)(intptr_t)v : CARRYON_OK;
}

/** Out-string wrapper tail: record the engine rc thread-locally, then hand
 *  back the copied bytes (NULL on error). */
static jbyteArray out_string_result(JNIEnv *env, jint rc, char *out) {
    set_out_rc(rc);
    if (rc != CARRYON_OK) return NULL;
    return out_bytes(env, out);
}

/** Write one engine-produced bool into a Kotlin BooleanArray(1) slot. */
static void put_bool(JNIEnv *env, jbooleanArray out, jboolean v) {
    if (out) (*env)->SetBooleanArrayRegion(env, out, 0, 1, &v);
}

/** *len-query pre-check (MEMORY SAFETY). The engine honors the ABI contract
 *  ("buf has *len bytes", see carryon.h) and copies exactly `required` bytes
 *  once cap >= required, so the SHIM must tie the caller-declared cap to the
 *  real JVM array length — this is the only place it is visible. Rejects:
 *  NULL len, a len array shorter than 1 element, a negative cap, and a cap
 *  larger than the real buf length. Returns CARRYON_OK with the validated cap
 *  in *cap_out; otherwise an ABI error code and NOTHING is written (a
 *  mismatched (buf, len) pair would otherwise be a native out-of-bounds
 *  write past the JVM array backing store, not a catchable exception). */
static jint len_precheck(JNIEnv *env, jbyteArray buf, jlongArray len,
                         jlong *cap_out) {
    if (!len) return CARRYON_ERR_NULL_ARG;
    if ((*env)->GetArrayLength(env, len) < 1) return CARRYON_ERR_NULL_ARG;
    jlong *l = (*env)->GetLongArrayElements(env, len, NULL);
    if (!l) return CARRYON_ERR_NULL_ARG;
    jlong v = l[0];
    (*env)->ReleaseLongArrayElements(env, len, l, JNI_ABORT);
    if (v < 0) return CARRYON_ERR_BUFFER_TOO_SMALL;
    if (buf && v > (jlong)(*env)->GetArrayLength(env, buf))
        return CARRYON_ERR_BUFFER_TOO_SMALL;
    *cap_out = v;
    return CARRYON_OK;
}

/* ---- ABI / lifetime ----------------------------------------------------- */

/* carryon_abi_version(&major, &minor) -> jintArray(major, minor). */
JNIEXPORT jintArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_abiVersion(JNIEnv *env, jobject thiz) {
    (void)thiz;
    uint32_t major = 0, minor = 0;
    carryon_abi_version(&major, &minor);
    jint v[2] = { (jint)major, (jint)minor };
    jintArray arr = (*env)->NewIntArray(env, 2);
    if (arr) (*env)->SetIntArrayRegion(env, arr, 0, 2, v);
    return arr;
}

/* carryon_last_error(buf, &len) — *len query protocol (see header comment). */
JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_lastError(JNIEnv *env, jobject thiz,
                                              jbyteArray buf, jlongArray len) {
    (void)thiz;
    jlong cap_in = 0;
    jint prc = len_precheck(env, buf, len, &cap_in);
    if (prc != CARRYON_OK) return prc;
    jlong *l = (*env)->GetLongArrayElements(env, len, NULL);
    if (!l) return CARRYON_ERR_NULL_ARG;
    size_t cap = (size_t)cap_in;
    uint8_t *bp = NULL;
    jbyte *be = NULL;
    if (buf) {
        be = (*env)->GetByteArrayElements(env, buf, NULL);
        bp = (uint8_t *)be;
    }
    jint rc = carryon_last_error(bp, &cap);
    l[0] = (jlong)cap;
    if (be) (*env)->ReleaseByteArrayElements(env, buf, be, 0);
    (*env)->ReleaseLongArrayElements(env, len, l, 0);
    return rc;
}

/* The engine rc of the last out-string call ON THIS THREAD (see
 * out_string_result). Kotlin reads it immediately after a jbyteArray wrapper
 * returned NULL — the signature itself cannot carry the code. */
JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_lastOutRc(JNIEnv *env, jobject thiz) {
    (void)env; (void)thiz;
    return get_out_rc();
}

/* Out-strings are translated shim-side (out_bytes), so carryon_string_free never
 * crosses to Kotlin. Mirrored here as a no-op guard only if ever needed. */

/* ---- Core --------------------------------------------------------------- */

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreOpen(JNIEnv *env, jobject thiz,
                                             jstring data_dir) {
    (void)thiz;
    const char *dir = borrow_utf(env, data_dir);
    CarryonCore *core = dir ? carryon_core_open(dir) : NULL;
    release_utf(env, data_dir, dir);
    return p2h(core);
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreFree(JNIEnv *env, jobject thiz, jlong core) {
    (void)env; (void)thiz;
    carryon_core_free((CarryonCore *)h2p(core));
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreRecoveryReportJson(JNIEnv *env, jobject thiz,
                                                           jlong core) {
    (void)thiz;
    char *out = NULL;
    jint rc = carryon_core_recovery_report_json((CarryonCore *)h2p(core), &out);
    return out_string_result(env, rc, out);
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreSetChunkSize(JNIEnv *env, jobject thiz,
                                                     jlong core, jlong bytes) {
    (void)env; (void)thiz;
    return carryon_core_set_chunk_size((CarryonCore *)h2p(core), (uint64_t)bytes);
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreSetBudget(JNIEnv *env, jobject thiz,
                                                  jlong core, jstring budget_json) {
    (void)thiz;
    const char *j = borrow_utf(env, budget_json);
    jint rc = j ? carryon_core_set_budget((CarryonCore *)h2p(core), j)
                : CARRYON_ERR_NULL_ARG;
    release_utf(env, budget_json, j);
    return rc;
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreSetForeground(JNIEnv *env, jobject thiz,
                                                      jlong core, jboolean fg) {
    (void)env; (void)thiz;
    return carryon_core_set_foreground((CarryonCore *)h2p(core), fg == JNI_TRUE);
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_coreRequestSuspend(JNIEnv *env, jobject thiz,
                                                       jlong core) {
    (void)env; (void)thiz;
    return carryon_core_request_suspend((CarryonCore *)h2p(core));
}

/* ---- Adapters / sessions ------------------------------------------------ */

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_registerAdapter(JNIEnv *env, jobject thiz,
                                                    jlong core, jstring adapter_id,
                                                    jstring params_json) {
    (void)thiz;
    const char *aid = borrow_utf(env, adapter_id);
    const char *pj = borrow_utf(env, params_json);
    char *out = NULL;
    jint rc = (aid && pj)
        ? carryon_register_adapter((CarryonCore *)h2p(core), aid, pj, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, adapter_id, aid);
    release_utf(env, params_json, pj);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_createSession(JNIEnv *env, jobject thiz,
                                                  jlong core,
                                                  jstring create_req_json) {
    (void)thiz;
    const char *req = borrow_utf(env, create_req_json);
    char *out = NULL;
    jint rc = req
        ? carryon_create_session((CarryonCore *)h2p(core), req, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, create_req_json, req);
    return out_string_result(env, rc, out);
}

/* ---- Prepare / carry ---------------------------------------------------- */

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_createCut(JNIEnv *env, jobject thiz,
                                              jlong core, jstring session_id) {
    (void)thiz;
    const char *sid = borrow_utf(env, session_id);
    char *out = NULL;
    jint rc = sid
        ? carryon_create_cut((CarryonCore *)h2p(core), sid, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, session_id, sid);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_listAvailableActions(JNIEnv *env, jobject thiz,
                                                         jlong core,
                                                         jstring cut_json,
                                                         jstring classes_json) {
    (void)thiz;
    const char *cut = borrow_utf(env, cut_json);
    const char *classes = borrow_utf(env, classes_json);
    char *out = NULL;
    jint rc = (cut && classes)
        ? carryon_list_available_actions((CarryonCore *)h2p(core), cut, classes, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, cut_json, cut);
    release_utf(env, classes_json, classes);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_executeActionAs(JNIEnv *env, jobject thiz,
                                                    jlong core, jstring cut_json,
                                                    jstring action_req_json,
                                                    jstring adapter_id) {
    (void)thiz;
    const char *cut = borrow_utf(env, cut_json);
    const char *req = borrow_utf(env, action_req_json);
    const char *aid = borrow_utf(env, adapter_id);
    char *out = NULL;
    jint rc = (cut && req && aid)
        ? carryon_execute_action_as((CarryonCore *)h2p(core), cut, req, aid, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, cut_json, cut);
    release_utf(env, action_req_json, req);
    release_utf(env, adapter_id, aid);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_executeAction(JNIEnv *env, jobject thiz,
                                                  jlong core, jstring cut_json,
                                                  jstring action_req_json) {
    (void)thiz;
    const char *cut = borrow_utf(env, cut_json);
    const char *req = borrow_utf(env, action_req_json);
    char *out = NULL;
    jint rc = (cut && req)
        ? carryon_execute_action((CarryonCore *)h2p(core), cut, req, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, cut_json, cut);
    release_utf(env, action_req_json, req);
    return out_string_result(env, rc, out);
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_hasObjectHex(JNIEnv *env, jobject thiz,
                                                 jlong core, jstring content_hash,
                                                 jbooleanArray out) {
    (void)thiz;
    const char *hash = borrow_utf(env, content_hash);
    bool present = false;
    jint rc = hash
        ? carryon_has_object_hex((CarryonCore *)h2p(core), hash, &present)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, content_hash, hash);
    if (rc == CARRYON_OK) put_bool(env, out, present ? JNI_TRUE : JNI_FALSE);
    return rc;
}

/* ---- Net: identity / trust / pairing ------------------------------------ */

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_identityGenerate(JNIEnv *env, jobject thiz,
                                                     jstring name) {
    (void)thiz;
    const char *n = borrow_utf(env, name);
    CarryonIdentity *id = n ? carryon_identity_generate(n) : NULL;
    release_utf(env, name, n);
    return p2h(id);
}

/* SECRET-adjacent: see the file header. Kotlin declares these `internal` and the
 * app-level API never surfaces the bytes — pin hex only (§22.3, EVD-006). */

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_identityCertDer(JNIEnv *env, jobject thiz,
                                                    jlong id, jbyteArray buf,
                                                    jlongArray len) {
    (void)thiz;
    jlong cap_in = 0;
    jint prc = len_precheck(env, buf, len, &cap_in);
    if (prc != CARRYON_OK) return prc;
    jlong *l = (*env)->GetLongArrayElements(env, len, NULL);
    if (!l) return CARRYON_ERR_NULL_ARG;
    size_t cap = (size_t)cap_in;
    jbyte *be = buf ? (*env)->GetByteArrayElements(env, buf, NULL) : NULL;
    jint rc = carryon_identity_cert_der((const CarryonIdentity *)h2p(id),
                                        (uint8_t *)be, &cap);
    l[0] = (jlong)cap;
    if (be) (*env)->ReleaseByteArrayElements(env, buf, be, 0);
    (*env)->ReleaseLongArrayElements(env, len, l, 0);
    return rc;
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_identityKeyDer(JNIEnv *env, jobject thiz,
                                                   jlong id, jbyteArray buf,
                                                   jlongArray len) {
    (void)thiz;
    jlong cap_in = 0;
    jint prc = len_precheck(env, buf, len, &cap_in);
    if (prc != CARRYON_OK) return prc;
    jlong *l = (*env)->GetLongArrayElements(env, len, NULL);
    if (!l) return CARRYON_ERR_NULL_ARG;
    size_t cap = (size_t)cap_in;
    jbyte *be = buf ? (*env)->GetByteArrayElements(env, buf, NULL) : NULL;
    jint rc = carryon_identity_key_der((const CarryonIdentity *)h2p(id),
                                       (uint8_t *)be, &cap);
    l[0] = (jlong)cap;
    if (be) (*env)->ReleaseByteArrayElements(env, buf, be, 0);
    (*env)->ReleaseLongArrayElements(env, len, l, 0);
    return rc;
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_identityPinHex(JNIEnv *env, jobject thiz,
                                                   jlong id) {
    (void)thiz;
    char *out = NULL;
    jint rc = carryon_identity_pin_hex((const CarryonIdentity *)h2p(id), &out);
    return out_string_result(env, rc, out);
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_identityFree(JNIEnv *env, jobject thiz, jlong id) {
    (void)env; (void)thiz;
    carryon_identity_free((CarryonIdentity *)h2p(id));
}

/* pair_devices -> `{local_trust, remote_trust}` JSON (abi_net.rs). */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_pairDevices(JNIEnv *env, jobject thiz,
                                                jlong a, jlong b, jstring utc) {
    (void)thiz;
    const char *u = borrow_utf(env, utc);
    char *out = NULL;
    jint rc = u
        ? carryon_pair_devices((const CarryonIdentity *)h2p(a),
                               (const CarryonIdentity *)h2p(b), u, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, utc, u);
    return out_string_result(env, rc, out);
}

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_trustFromJson(JNIEnv *env, jobject thiz,
                                                  jstring json) {
    (void)thiz;
    const char *j = borrow_utf(env, json);
    CarryonTrust *t = j ? carryon_trust_from_json(j) : NULL;
    release_utf(env, json, j);
    return p2h(t);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_trustToJson(JNIEnv *env, jobject thiz,
                                                jlong trust) {
    (void)thiz;
    char *out = NULL;
    jint rc = carryon_trust_to_json((const CarryonTrust *)h2p(trust), &out);
    return out_string_result(env, rc, out);
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_trustPair(JNIEnv *env, jobject thiz,
                                              jlong trust, jstring name,
                                              jstring pin_hex, jstring utc) {
    (void)thiz;
    const char *n = borrow_utf(env, name);
    const char *p = borrow_utf(env, pin_hex);
    const char *u = borrow_utf(env, utc);
    jint rc = (n && p && u)
        ? carryon_trust_pair((CarryonTrust *)h2p(trust), n, p, u)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, name, n);
    release_utf(env, pin_hex, p);
    release_utf(env, utc, u);
    return rc;
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_trustIsTrusted(JNIEnv *env, jobject thiz,
                                                   jlong trust, jstring pin_hex,
                                                   jbooleanArray out) {
    (void)thiz;
    const char *p = borrow_utf(env, pin_hex);
    bool trusted = false;
    jint rc = p
        ? carryon_trust_is_trusted((CarryonTrust *)h2p(trust), p, &trusted)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, pin_hex, p);
    if (rc == CARRYON_OK) put_bool(env, out, trusted ? JNI_TRUE : JNI_FALSE);
    return rc;
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_trustRevoke(JNIEnv *env, jobject thiz,
                                                jlong trust, jstring pin_hex,
                                                jbooleanArray out) {
    (void)thiz;
    const char *p = borrow_utf(env, pin_hex);
    bool revoked = false;
    jint rc = p
        ? carryon_trust_revoke((CarryonTrust *)h2p(trust), p, &revoked)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, pin_hex, p);
    if (rc == CARRYON_OK) put_bool(env, out, revoked ? JNI_TRUE : JNI_FALSE);
    return rc;
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_trustFree(JNIEnv *env, jobject thiz, jlong trust) {
    (void)env; (void)thiz;
    carryon_trust_free((CarryonTrust *)h2p(trust));
}

/* ---- Net: listener / session -------------------------------------------- */

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_listenerBind(JNIEnv *env, jobject thiz,
                                                 jstring addr) {
    (void)thiz;
    const char *a = borrow_utf(env, addr);
    CarryonListener *l = a ? carryon_listener_bind(a) : NULL;
    release_utf(env, addr, a);
    return p2h(l);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_listenerAddr(JNIEnv *env, jobject thiz,
                                                 jlong listener) {
    (void)thiz;
    char *out = NULL;
    jint rc = carryon_listener_addr((const CarryonListener *)h2p(listener), &out);
    return out_string_result(env, rc, out);
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_listenerFree(JNIEnv *env, jobject thiz,
                                                 jlong listener) {
    (void)env; (void)thiz;
    carryon_listener_free((CarryonListener *)h2p(listener));
}

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionConnect(JNIEnv *env, jobject thiz,
                                                   jstring addr, jlong id,
                                                   jlong trust) {
    (void)thiz;
    const char *a = borrow_utf(env, addr);
    CarryonSession *s = a
        ? carryon_session_connect(a, (const CarryonIdentity *)h2p(id),
                                  (const CarryonTrust *)h2p(trust))
        : NULL;
    release_utf(env, addr, a);
    return p2h(s);
}

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionAccept(JNIEnv *env, jobject thiz,
                                                  jlong listener, jlong id,
                                                  jlong trust) {
    (void)env; (void)thiz;
    CarryonSession *s = carryon_session_accept((const CarryonListener *)h2p(listener),
                                               (const CarryonIdentity *)h2p(id),
                                               (const CarryonTrust *)h2p(trust));
    return p2h(s);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionClientNegotiate(JNIEnv *env, jobject thiz,
                                                           jlong session,
                                                           jstring features_json) {
    (void)thiz;
    const char *f = borrow_utf(env, features_json);
    char *out = NULL;
    jint rc = f
        ? carryon_session_client_negotiate((CarryonSession *)h2p(session), f, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, features_json, f);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionServerNegotiate(JNIEnv *env, jobject thiz,
                                                           jlong session,
                                                           jstring features_json) {
    (void)thiz;
    const char *f = borrow_utf(env, features_json);
    char *out = NULL;
    jint rc = f
        ? carryon_session_server_negotiate((CarryonSession *)h2p(session), f, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, features_json, f);
    return out_string_result(env, rc, out);
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionFree(JNIEnv *env, jobject thiz,
                                                jlong session) {
    (void)env; (void)thiz;
    carryon_session_free((CarryonSession *)h2p(session));
}

/* ---- Net: identity restore (PLAT-002) + QR pairing (ABI 1.2, §18.4) ------ */

/* carryon_identity_from_der — the PLAT-002 restore path: the app rehydrates
 * its durable identity from Keystore-held DER bytes (stable pin across
 * launches). The key DER arrives from Keystore-decrypted bytes only. */
JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_identityFromDer(JNIEnv *env, jobject thiz,
                                                    jstring name,
                                                    jbyteArray cert,
                                                    jbyteArray key) {
    (void)thiz;
    const char *n = borrow_utf(env, name);
    if (!n || !cert || !key) {
        release_utf(env, name, n);
        return 0;
    }
    jbyte *ce = (*env)->GetByteArrayElements(env, cert, NULL);
    jbyte *ke = (*env)->GetByteArrayElements(env, key, NULL);
    jsize cn = (*env)->GetArrayLength(env, cert);
    jsize kn = (*env)->GetArrayLength(env, key);
    CarryonIdentity *id = (ce && ke)
        ? carryon_identity_from_der(n, (const uint8_t *)ce, (size_t)cn,
                                    (const uint8_t *)ke, (size_t)kn)
        : NULL;
    if (ce) (*env)->ReleaseByteArrayElements(env, cert, ce, JNI_ABORT);
    if (ke) (*env)->ReleaseByteArrayElements(env, key, ke, JNI_ABORT);
    release_utf(env, name, n);
    return p2h(id);
}

/* carryon_pairing_payload_format — §18.4.1: the destination's QR URI. */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_pairingPayloadFormat(JNIEnv *env, jobject thiz,
                                                         jstring device_name,
                                                         jstring addr,
                                                         jstring pin_hex,
                                                         jstring code) {
    (void)thiz;
    const char *n = borrow_utf(env, device_name);
    const char *a = borrow_utf(env, addr);
    const char *k = borrow_utf(env, pin_hex);
    const char *r = borrow_utf(env, code);
    char *out = NULL;
    jint rc = (n && a && k && r)
        ? carryon_pairing_payload_format(n, a, k, r, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, device_name, n);
    release_utf(env, addr, a);
    release_utf(env, pin_hex, k);
    release_utf(env, code, r);
    return out_string_result(env, rc, out);
}

/* carryon_pairing_payload_parse — §18.4.2: strict, fail-closed parse. */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_pairingPayloadParse(JNIEnv *env, jobject thiz,
                                                        jstring uri) {
    (void)thiz;
    const char *u = borrow_utf(env, uri);
    char *out = NULL;
    jint rc = u ? carryon_pairing_payload_parse(u, &out) : CARRYON_ERR_NULL_ARG;
    release_utf(env, uri, u);
    return out_string_result(env, rc, out);
}

/* carryon_pairing_verification_string — §18.4.4: both devices' short string. */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_pairingVerificationString(JNIEnv *env, jobject thiz,
                                                              jstring pin_a_hex,
                                                              jstring pin_b_hex) {
    (void)thiz;
    const char *a = borrow_utf(env, pin_a_hex);
    const char *b = borrow_utf(env, pin_b_hex);
    char *out = NULL;
    jint rc = (a && b)
        ? carryon_pairing_verification_string(a, b, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, pin_a_hex, a);
    release_utf(env, pin_b_hex, b);
    return out_string_result(env, rc, out);
}

/* carryon_pairing_code — the six-digit code shown with the QR (§18.4.1-2). */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_pairingCode(JNIEnv *env, jobject thiz,
                                                jstring pin_hex, jlong seed) {
    (void)thiz;
    const char *k = borrow_utf(env, pin_hex);
    char *out = NULL;
    jint rc = k ? carryon_pairing_code(k, (uint64_t)seed, &out) : CARRYON_ERR_NULL_ARG;
    release_utf(env, pin_hex, k);
    return out_string_result(env, rc, out);
}

/* ---- Net: provisional pairing channel (§18.4.3) -------------------------- */

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionConnectPairing(JNIEnv *env, jobject thiz,
                                                          jstring addr, jlong id) {
    (void)thiz;
    const char *a = borrow_utf(env, addr);
    CarryonSession *s = a
        ? carryon_session_connect_pairing(a, (const CarryonIdentity *)h2p(id))
        : NULL;
    release_utf(env, addr, a);
    return p2h(s);
}

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionAcceptPairing(JNIEnv *env, jobject thiz,
                                                         jlong listener, jlong id) {
    (void)env; (void)thiz;
    CarryonSession *s = carryon_session_accept_pairing(
        (const CarryonListener *)h2p(listener), (const CarryonIdentity *)h2p(id));
    return p2h(s);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionPeerPinHex(JNIEnv *env, jobject thiz,
                                                      jlong session) {
    (void)thiz;
    char *out = NULL;
    jint rc = carryon_session_peer_pin_hex((const CarryonSession *)h2p(session), &out);
    return out_string_result(env, rc, out);
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionIsPairing(JNIEnv *env, jobject thiz,
                                                     jlong session,
                                                     jbooleanArray out) {
    (void)thiz;
    bool pairing = false;
    jint rc = carryon_session_is_pairing((const CarryonSession *)h2p(session), &pairing);
    if (rc == CARRYON_OK) put_bool(env, out, pairing ? JNI_TRUE : JNI_FALSE);
    return rc;
}


/* ---- Discovery (ABI 1.2 §18.3) ------------------------------------------- */

/* Handle-based discovery entries (the TXT encode/validate pair lives above).
 * Advertiser/browser handles are opaque jlong exactly like every other
 * engine handle; the snapshot is an out-string (the same ownership rule). */

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryAdvertiseStart(JNIEnv *env, jobject thiz,
                                                       jstring instance_name,
                                                       jint port,
                                                       jstring txt_pairs_json) {
    (void)thiz;
    const char *n = borrow_utf(env, instance_name);
    const char *t = borrow_utf(env, txt_pairs_json);
    CarryonDiscoveryAdvertiser *adv = (n && t)
        ? carryon_discovery_advertise_start(n, (uint16_t)port, t)
        : NULL;
    release_utf(env, instance_name, n);
    release_utf(env, txt_pairs_json, t);
    return p2h(adv);
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryAdvertiseStop(JNIEnv *env, jobject thiz,
                                                       jlong adv) {
    (void)env; (void)thiz;
    carryon_discovery_advertise_stop((CarryonDiscoveryAdvertiser *)h2p(adv));
}

JNIEXPORT jlong JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryBrowseStart(JNIEnv *env, jobject thiz) {
    (void)env; (void)thiz;
    return p2h(carryon_discovery_browse_start());
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryBrowseSnapshot(JNIEnv *env, jobject thiz,
                                                        jlong browser) {
    (void)thiz;
    char *out = NULL;
    jint rc = carryon_discovery_browse_snapshot(
        (const CarryonDiscoveryBrowser *)h2p(browser), &out);
    return out_string_result(env, rc, out);
}

JNIEXPORT void JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryBrowseStop(JNIEnv *env, jobject thiz,
                                                    jlong browser) {
    (void)env; (void)thiz;
    carryon_discovery_browse_stop((CarryonDiscoveryBrowser *)h2p(browser));
}

/* ---- Transfer (handoff) -------------------------------------------------- */

/* Blocking: runs until the peer's import_cut finishes. Kotlin calls this on a
 * dedicated IO coroutine; DemoRunner pairs it with importCut on the other core. */
JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_serveCut(JNIEnv *env, jobject thiz,
                                             jlong core, jlong session) {
    (void)env; (void)thiz;
    return carryon_serve_cut((CarryonCore *)h2p(core), (CarryonSession *)h2p(session));
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_importCut(JNIEnv *env, jobject thiz,
                                              jlong core, jlong session,
                                              jstring remote_session,
                                              jlong remote_cut) {
    (void)thiz;
    const char *rs = borrow_utf(env, remote_session);
    char *out = NULL;
    jint rc = rs
        ? carryon_import_cut((CarryonCore *)h2p(core), (CarryonSession *)h2p(session),
                             rs, (uint64_t)remote_cut, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, remote_session, rs);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_resumeImport(JNIEnv *env, jobject thiz,
                                                 jlong core, jlong session,
                                                 jstring resume_token_json) {
    (void)thiz;
    const char *tok = borrow_utf(env, resume_token_json);
    char *out = NULL;
    jint rc = tok
        ? carryon_resume_import((CarryonCore *)h2p(core), (CarryonSession *)h2p(session),
                                tok, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, resume_token_json, tok);
    return out_string_result(env, rc, out);
}

/* Object read via the *len query protocol; bytes verified against the declared
 * content_hash by the core before publish (CORE-004 two-phase invariant). */
JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_readObject(JNIEnv *env, jobject thiz,
                                               jlong core, jstring content_hash,
                                               jbyteArray buf, jlongArray len) {
    (void)thiz;
    jlong cap_in = 0;
    jint prc = len_precheck(env, buf, len, &cap_in);
    if (prc != CARRYON_OK) return prc;
    const char *hash = borrow_utf(env, content_hash);
    if (!hash) return CARRYON_ERR_NULL_ARG;
    jlong *l = (*env)->GetLongArrayElements(env, len, NULL);
    if (!l) {
        release_utf(env, content_hash, hash);
        return CARRYON_ERR_NULL_ARG;
    }
    size_t cap = (size_t)cap_in;
    jbyte *be = buf ? (*env)->GetByteArrayElements(env, buf, NULL) : NULL;
    jint rc = carryon_read_object((const CarryonCore *)h2p(core), hash,
                                  (uint8_t *)be, &cap);
    l[0] = (jlong)cap;
    if (be) (*env)->ReleaseByteArrayElements(env, buf, be, 0);
    (*env)->ReleaseLongArrayElements(env, len, l, 0);
    release_utf(env, content_hash, hash);
    return rc;
}

/* -> LongArray(2): [sent, recv]. Live transport counters (MEASURED, §5.6). */
JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_sessionBytes(JNIEnv *env, jobject thiz,
                                                 jlong session, jlongArray out) {
    (void)thiz;
    if (!out) return CARRYON_ERR_NULL_ARG;
    if ((*env)->GetArrayLength(env, out) < 2) return CARRYON_ERR_BUFFER_TOO_SMALL;
    jlong *o = (*env)->GetLongArrayElements(env, out, NULL);
    if (!o) return CARRYON_ERR_NULL_ARG;
    uint64_t sent = 0, recv = 0;
    jint rc = carryon_session_bytes((const CarryonSession *)h2p(session), &sent, &recv);
    if (rc == CARRYON_OK) {
        o[0] = (jlong)sent;
        o[1] = (jlong)recv;
    }
    (*env)->ReleaseLongArrayElements(env, out, o, 0);
    return rc;
}

/* ---- Authority transfer (L4, §21.2/§21.3) -------------------------------- */

/* Blocking converse of requestAuthorityTransfer (propose -> await accept ->
 * commit relinquishment). DemoRunner runs both on separate IO coroutines. */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_serveAuthorityTransfer(JNIEnv *env, jobject thiz,
                                                           jlong core, jlong session,
                                                           jstring session_id,
                                                           jlong cut_number) {
    (void)thiz;
    const char *sid = borrow_utf(env, session_id);
    char *out = NULL;
    jint rc = sid
        ? carryon_serve_authority_transfer((CarryonCore *)h2p(core),
                                           (CarryonSession *)h2p(session), sid,
                                           (uint64_t)cut_number, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, session_id, sid);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_requestAuthorityTransfer(JNIEnv *env, jobject thiz,
                                                             jlong core, jlong session,
                                                             jstring mirror_session_id,
                                                             jstring adapter_id) {
    (void)thiz;
    const char *msid = borrow_utf(env, mirror_session_id);
    const char *aid = borrow_utf(env, adapter_id);
    char *out = NULL;
    jint rc = (msid && aid)
        ? carryon_request_authority_transfer((CarryonCore *)h2p(core),
                                             (CarryonSession *)h2p(session), msid,
                                             aid, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, mirror_session_id, msid);
    release_utf(env, adapter_id, aid);
    return out_string_result(env, rc, out);
}

/* -> LongArray(1): fresh epoch (§21.3 recovery). */
JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_recoverAuthority(JNIEnv *env, jobject thiz,
                                                     jlong core,
                                                     jstring session_id,
                                                     jlongArray out) {
    (void)thiz;
    const char *sid = borrow_utf(env, session_id);
    if (!sid || !out) {
        release_utf(env, session_id, sid);
        return CARRYON_ERR_NULL_ARG;
    }
    if ((*env)->GetArrayLength(env, out) < 1) {
        release_utf(env, session_id, sid);
        return CARRYON_ERR_BUFFER_TOO_SMALL;
    }
    jlong *o = (*env)->GetLongArrayElements(env, out, NULL);
    if (!o) {
        release_utf(env, session_id, sid);
        return CARRYON_ERR_NULL_ARG;
    }
    uint64_t epoch = 0;
    jint rc = carryon_recover_authority((CarryonCore *)h2p(core), sid, &epoch);
    if (rc == CARRYON_OK) o[0] = (jlong)epoch;
    (*env)->ReleaseLongArrayElements(env, out, o, 0);
    release_utf(env, session_id, sid);
    return rc;
}

JNIEXPORT jint JNICALL
Java_dev_carryon_kit_CarryOnFfi_mayMutate(JNIEnv *env, jobject thiz,
                                              jlong core, jstring session_id,
                                              jbooleanArray out) {
    (void)thiz;
    const char *sid = borrow_utf(env, session_id);
    bool may = false;
    jint rc = sid
        ? carryon_may_mutate((const CarryonCore *)h2p(core), sid, &may)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, session_id, sid);
    if (rc == CARRYON_OK) put_bool(env, out, may ? JNI_TRUE : JNI_FALSE);
    return rc;
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_mirrorSessionId(JNIEnv *env, jobject thiz,
                                                    jstring remote_session) {
    (void)thiz;
    const char *rs = borrow_utf(env, remote_session);
    char *out = NULL;
    jint rc = rs
        ? carryon_mirror_session_id(rs, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, remote_session, rs);
    return out_string_result(env, rc, out);
}

/* ---- Evidence ------------------------------------------------------------ */

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_exportEvidence(JNIEnv *env, jobject thiz,
                                                   jlong core, jstring session_id) {
    (void)thiz;
    const char *sid = borrow_utf(env, session_id);
    char *out = NULL;
    jint rc = sid
        ? carryon_export_evidence((CarryonCore *)h2p(core), sid, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, session_id, sid);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_exportEvidenceWith(JNIEnv *env, jobject thiz,
                                                       jlong core,
                                                       jstring session_id,
                                                       jstring extra_metrics_json) {
    (void)thiz;
    const char *sid = borrow_utf(env, session_id);
    const char *extra = borrow_utf(env, extra_metrics_json);
    char *out = NULL;
    jint rc = (sid && extra)
        ? carryon_export_evidence_with((CarryonCore *)h2p(core), sid, extra, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, session_id, sid);
    release_utf(env, extra_metrics_json, extra);
    return out_string_result(env, rc, out);
}

/* Verifies a bundle FROM DISK path; bundle content is parsed, never executed
 * (EVD-005). The caller owns the bundle file (app storage). */
JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_verifyEvidence(JNIEnv *env, jobject thiz,
                                                   jlong core, jstring bundle_path) {
    (void)thiz;
    const char *path = borrow_utf(env, bundle_path);
    char *out = NULL;
    jint rc = path
        ? carryon_verify_evidence((const CarryonCore *)h2p(core), path, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, bundle_path, path);
    return out_string_result(env, rc, out);
}

/* ---- Discovery (spec §18.3, stage 4; ABI 1.2) -----------------------------
 * Android's NSD responder registers/browses natively (app layer), but the TXT
 * contract is the ENGINE's: advertise TXT is built by
 * carryon_discovery_txt_encode, and every received record is validated by
 * carryon_discovery_txt_validate (fail closed — E_PLATFORM_DISCOVERY drops the
 * peer, never repairs it). The browse/advertise engine entries are
 * desktop-side (mdns-sd) and are NOT bridged here: NSD owns them on Android.
 */

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryTxtEncode(JNIEnv *env, jobject thiz,
                                                       jstring fields_json) {
    (void)thiz;
    const char *fields = borrow_utf(env, fields_json);
    char *out = NULL;
    jint rc = fields
        ? carryon_discovery_txt_encode(fields, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, fields_json, fields);
    return out_string_result(env, rc, out);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_carryon_kit_CarryOnFfi_discoveryTxtValidate(JNIEnv *env, jobject thiz,
                                                         jstring pairs_json) {
    (void)thiz;
    const char *pairs = borrow_utf(env, pairs_json);
    char *out = NULL;
    jint rc = pairs
        ? carryon_discovery_txt_validate(pairs, &out)
        : CARRYON_ERR_NULL_ARG;
    release_utf(env, pairs_json, pairs);
    return out_string_result(env, rc, out);
}
