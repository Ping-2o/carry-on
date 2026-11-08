/*
 * CarryOnFfi.kt — raw 1:1 mirror of the REAL engine C ABI.
 *
 * Binds crates/carryon-ffi/include/carryon.h (ABI 1.1, append-only) through the
 * app-local JNI shim (app/src/main/cpp/carryon_jni.c). No Rust changes; no new
 * `#[no_mangle]` symbols (plan.md hard rule).
 *
 * Conventions mirrored from the C ABI (see carryon.h header comment):
 *  - Fallible calls return Int: 0 = CARRYON_OK, negative = FFI-local error,
 *    positive = core error (family_base + ordinal). Message via [lastError]
 *    (thread-local engine state; fetch it on the SAME thread as the failing call).
 *  - Pointer-returning calls return 0L on failure.
 *  - Out-strings arrive as UTF-8 jbyteArray copies; the shim frees the original
 *    with `carryon_string_free`, so ownership never crosses to Kotlin.
 *  - Out-buffers (cert/key DER, last_error, read_object) use the *len query
 *    protocol: pass buf=null, then allocate to len[0] and call again
 *    (CARRYON_ERR_BUFFER_TOO_SMALL reports the required size in len[0]).
 *
 * SECRETS (§22.3, EVD-006): [identityKeyDer]/[identityCertDer] mirror the ABI for
 * future secure-storage persistence (PLAT-002 is TARGET). The app-level wrapper
 * (CarryonCore.kt) exposes ONLY the SHA-256 pin hex; the key DER is never logged,
 * displayed, persisted here, or exported.
 *
 * Threading: the engine is callable from any thread; blocking calls (serveCut,
 * serveAuthorityTransfer) MUST run off the main thread (DemoRunner uses
 * Dispatchers.IO coroutines — same discipline as the macOS wrapper).
 */
package dev.carryon.kit

/** Raw external functions. Call through the typed wrappers in CarryonCore.kt
 *  (Core/Identity/Trust/Listener/Session) instead. */
object CarryOnFfi {

    init {
        // libcarryon_jni.so (the shim) links libcarryon_ffi.so (the engine);
        // both ship in the APK's native lib dir, so the linker resolves the
        // engine dependency at load time.
        System.loadLibrary("carryon_jni")
    }

    // ---- ABI / lifetime ----------------------------------------------------

    /** -> IntArray(major, minor); check against CARRYON_ABI 1.1 at load. */
    external fun abiVersion(): IntArray

    /** *len query protocol; message bytes are NUL-terminated UTF-8. */
    external fun lastError(buf: ByteArray?, len: LongArray): Int

    /** Engine rc of the last out-string call on THIS thread (shim thread-local;
     *  read immediately after a jbyteArray wrapper returned null — the JNI
     *  signature itself cannot carry the code). Same-thread discipline as
     *  lastError. */
    external fun lastOutRc(): Int

    // ---- Core ---------------------------------------------------------------

    external fun coreOpen(dataDir: String): Long
    external fun coreFree(core: Long)
    external fun coreRecoveryReportJson(core: Long): ByteArray?
    external fun coreSetChunkSize(core: Long, bytes: Long): Int
    external fun coreSetBudget(core: Long, budgetJson: String): Int
    external fun coreSetForeground(core: Long, fg: Boolean): Int
    external fun coreRequestSuspend(core: Long): Int

    // ---- Adapters / sessions -------------------------------------------------

    /** Selects a compiled-in adapter by id + JSON params (data, never behavior
     *  — spec §3.8). -> info JSON bytes, or null on error. */
    external fun registerAdapter(core: Long, adapterId: String, paramsJson: String): ByteArray?
    external fun createSession(core: Long, createReqJson: String): ByteArray?

    // ---- Prepare / carry ------------------------------------------------------

    external fun createCut(core: Long, sessionId: String): ByteArray?
    external fun listAvailableActions(core: Long, cutJson: String, classesJson: String): ByteArray?
    external fun executeActionAs(
        core: Long, cutJson: String, actionReqJson: String, adapterId: String
    ): ByteArray?
    external fun executeAction(core: Long, cutJson: String, actionReqJson: String): ByteArray?
    external fun hasObjectHex(core: Long, contentHash: String, out: BooleanArray): Int

    // ---- Net: identity / trust / pairing --------------------------------------

    external fun identityGenerate(name: String): Long

    // NOTE: certDer/keyDer are part of the ABI mirror (plan.md: external funs
    // mirror the C ABI; `internal` would mangle the JVM name and break JNI
    // resolution). The SECRET rule (§22.3, EVD-006) is enforced one layer up:
    // CarryonCore exposes ONLY the pin hex and never calls these; nothing in the
    // app may log, display, or persist the key DER until PLAT-002 (Keystore).
    /** *len protocol. App-level API must NOT surface the bytes (§22.3). */
    external fun identityCertDer(id: Long, buf: ByteArray?, len: LongArray): Int
    /** SECRET (§22.3). Mirror of the ABI only; see the note above. */
    external fun identityKeyDer(id: Long, buf: ByteArray?, len: LongArray): Int
    external fun identityPinHex(id: Long): ByteArray?
    external fun identityFree(id: Long)

    /** -> `{local_trust:{…},remote_trust:{…}}` JSON bytes. */
    external fun pairDevices(a: Long, b: Long, utc: String): ByteArray?
    external fun trustFromJson(json: String): Long
    external fun trustToJson(trust: Long): ByteArray?
    external fun trustPair(trust: Long, name: String, pinHex: String, utc: String): Int
    external fun trustIsTrusted(trust: Long, pinHex: String, out: BooleanArray): Int
    external fun trustRevoke(trust: Long, pinHex: String, out: BooleanArray): Int
    external fun trustFree(trust: Long)

    // ---- Net: identity restore (PLAT-002) + QR pairing (ABI 1.2, §18.4) ---------

    /** PLAT-002 restore: rehydrate the durable identity from Keystore-held
     * DER bytes (stable pin across launches, §18.4). */
    external fun identityFromDer(name: String, cert: ByteArray, key: ByteArray): Long
    /** §18.4.1: the destination's QR payload URI bytes. */
    external fun pairingPayloadFormat(
        deviceName: String, addr: String, pinHex: String, code: String
    ): ByteArray?
    /** §18.4.2: strict parse -> `{version,device_name,addr,pin_hex,code}` JSON bytes. */
    external fun pairingPayloadParse(uri: String): ByteArray?
    /** §18.4.4: the short verification string (6 hex) both devices display. */
    external fun pairingVerificationString(pinAHex: String, pinBHex: String): ByteArray?
    /** §18.4.1-2: the six-digit pairing code shown beside the QR. */
    external fun pairingCode(pinHex: String, seed: Long): ByteArray?

    // ---- Net: listener / session -----------------------------------------------

    external fun listenerBind(addr: String): Long
    external fun listenerAddr(listener: Long): ByteArray?
    external fun listenerFree(listener: Long)
    external fun sessionConnect(addr: String, id: Long, trust: Long): Long
    external fun sessionAccept(listener: Long, id: Long, trust: Long): Long
    /** §18.4.3 provisional pairing channel: pin recorded, not pre-trusted. */
    external fun sessionConnectPairing(addr: String, id: Long): Long
    /** §18.4.3 provisional accept: the connecting source's pin is recorded. */
    external fun sessionAcceptPairing(listener: Long, id: Long): Long
    /** The peer pin recorded on this session (public fingerprint). */
    external fun sessionPeerPinHex(session: Long): ByteArray?
    /** Whether this is the provisional pairing channel (content refused). */
    external fun sessionIsPairing(session: Long, out: BooleanArray): Int
    external fun sessionClientNegotiate(session: Long, featuresJson: String): ByteArray?
    external fun sessionServerNegotiate(session: Long, featuresJson: String): ByteArray?
    external fun sessionFree(session: Long)

    // ---- Discovery (spec §18.3, stage 4; ABI 1.2) -------------------------------
    // The ENGINE owns the TXT contract. Android's NSD responder registers and
    // browses natively (app layer), but the TXT it ADVERTISES is built by
    // discoveryTxtEncode and every record it RECEIVES is validated by
    // discoveryTxtValidate — fail closed: on CoreException the peer is dropped,
    // never repaired. The engine's own mdns advertise/browse entries are the
    // desktop path and stay unbridged here.

    /** fields JSON -> canonical "[[key,value],...]" pairs (pm id pr dc ch). */
    external fun discoveryTxtEncode(fieldsJson: String): ByteArray?

    /** received "[[key,value],...]" -> {"txt":{...},"ignored_keys":[...]}. */
    external fun discoveryTxtValidate(pairsJson: String): ByteArray?

    // ---- Discovery (ABI 1.2, §18.3) ------------------------------------------

    /** Start advertising `_carryon._tcp` with §18.3 minimal TXT (pairs from
     *  [discoveryTxtEncode]). -> advertiser handle, or 0 on failure. */
    external fun discoveryAdvertiseStart(instanceName: String, port: Int, txtPairsJson: String): Long

    /** Stop advertising; the handle is consumed. */
    external fun discoveryAdvertiseStop(adv: Long)

    /** Start browsing for `_carryon._tcp`. -> browser handle, or 0 on failure. */
    external fun discoveryBrowseStart(): Long

    /** Current validated peer snapshot (JSON bytes), or null on error. */
    external fun discoveryBrowseSnapshot(browser: Long): ByteArray?

    /** Stop browsing; the handle is consumed. */
    external fun discoveryBrowseStop(browser: Long)

    // ---- Transfer (handoff) -----------------------------------------------------

    /** BLOCKING: serves until the peer's import completes. Dispatchers.IO only. */
    external fun serveCut(core: Long, session: Long): Int

    /** -> outcome JSON: `{"completed":true,"mirror_cut":N}` |
     *  `{"suspended":true,"resume_token":{…}}` |
     *  `{"partial_budget":true,…}` (§6.8 — a NORMAL outcome). */
    external fun importCut(
        core: Long, session: Long, remoteSession: String, remoteCut: Long
    ): ByteArray?
    external fun resumeImport(
        core: Long, session: Long, resumeTokenJson: String
    ): ByteArray?
    external fun readObject(core: Long, contentHash: String, buf: ByteArray?, len: LongArray): Int

    /** -> LongArray(2): [sent, recv] live transport counters (MEASURED, §5.6). */
    external fun sessionBytes(session: Long, out: LongArray): Int

    // ---- Authority transfer (L4, §21.2/§21.3) ------------------------------------

    /** BLOCKING converse of [requestAuthorityTransfer]. Dispatchers.IO only. */
    external fun serveAuthorityTransfer(
        core: Long, session: Long, sessionId: String, cutNumber: Long
    ): ByteArray?
    external fun requestAuthorityTransfer(
        core: Long, session: Long, mirrorSessionId: String, adapterId: String
    ): ByteArray?
    external fun recoverAuthority(core: Long, sessionId: String, out: LongArray): Int
    external fun mayMutate(core: Long, sessionId: String, out: BooleanArray): Int
    external fun mirrorSessionId(remoteSession: String): ByteArray?

    // ---- Evidence ------------------------------------------------------------------

    external fun exportEvidence(core: Long, sessionId: String): ByteArray?
    external fun exportEvidenceWith(
        core: Long, sessionId: String, extraMetricsJson: String
    ): ByteArray?
    external fun verifyEvidence(core: Long, bundlePath: String): ByteArray?
}
