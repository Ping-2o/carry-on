/*
 * CarryonCore.kt — typed Kotlin wrappers over the raw ABI mirror (CarryOnFfi).
 *
 * What lives here (plan.md: "Core handle wrapper, error-code mirror, *len
 * protocol helpers, JSON out-string helpers"):
 *   - [Err]          the stable i32 code table, mirrored from
 *                      crates/carryon-ffi/src/errors.rs (append-only, ABI major 1).
 *   - [CoreException]  thrown with the engine's code + thread-local last_error
 *                      message (fetched on the SAME thread as the failed call).
 *   - [Core]/[Identity]/[Trust]/[Listener]/[Session]  handle wrappers; each is
 *                      Closeable and frees the engine handle exactly once.
 *   - *len protocol + out-string helpers ([sizedBytes], [requireOut],
 *                      [lastErrorMessage]) implementing the carryon.h conventions.
 *   - Parsers for engine JSON: [CutRef], [ImportOutcome], [RecoveryReport],
 *                      [TrustRecord], [PairingResult].
 *
 * SECRETS (§22.3, EVD-006): Identity exposes ONLY the SHA-256 pin hex. The cert/
 * key DER ABI entries in CarryOnFfi are deliberately NOT called anywhere in the
 * app; the key DER is never logged, displayed, or persisted.
 *
 * This file mirrors engine behavior only — labels follow §2: the demo built on
 * these wrappers is OBSERVED (LOCAL) evidence, never physical cross-device.
 */
package com.carryon.app.ffi

import org.json.JSONArray
import org.json.JSONObject
import java.io.Closeable

/* ---- error-code mirror (errors.rs) --------------------------------------- */

/** Stable ABI codes. 0 = OK; negative = FFI-local; positive = family_base +
 *  ordinal (§24 machine-readable codes at the boundary). */
object Err {
    const val OK = 0

    // FFI-local failures (negative).
    const val ERR_PANIC = -1
    const val ERR_NULL_ARG = -2
    const val ERR_BAD_UTF8 = -3
    const val ERR_BAD_JSON = -4
    const val ERR_BAD_HANDLE = -5
    const val ERR_BUFFER_TOO_SMALL = -6

    /** Sentinel for failures whose code the JNI shim could not carry across
     *  (pointer-returning entries); the message still comes from last_error. */
    const val UNKNOWN = Int.MIN_VALUE

    // Family bases.
    const val E_ADAPTER_BASE = 100
    const val E_SCHEMA_BASE = 200
    const val E_OBJECT_BASE = 300
    const val E_TRANSFER_BASE = 400
    const val E_BUDGET_BASE = 500
    const val E_ACTION_BASE = 600
    const val E_AUTH_BASE = 700
    const val E_MATH_BASE = 800
    const val E_INTERNAL_BASE = 900
    const val E_PROTO_BASE = 1000
    const val E_PLATFORM_BASE = 1100

    /** Stable name for a code — the Kotlin mirror of `code_of` (errors.rs). */
    fun name(code: Int): String = when (code) {
        OK -> "OK"
        ERR_PANIC -> "ERR_PANIC"
        ERR_NULL_ARG -> "ERR_NULL_ARG"
        ERR_BAD_UTF8 -> "ERR_BAD_UTF8"
        ERR_BAD_JSON -> "ERR_BAD_JSON"
        ERR_BAD_HANDLE -> "ERR_BAD_HANDLE"
        ERR_BUFFER_TOO_SMALL -> "ERR_BUFFER_TOO_SMALL"
        UNKNOWN -> "UNKNOWN"
        // Adapter (100..106).
        E_ADAPTER_BASE + 0 -> "E_ADAPTER_MISSING"
        E_ADAPTER_BASE + 1 -> "E_ADAPTER_INCOMPATIBLE"
        E_ADAPTER_BASE + 2 -> "E_ADAPTER_CRASHED"
        E_ADAPTER_BASE + 3 -> "E_ADAPTER_TIMEOUT"
        E_ADAPTER_BASE + 4 -> "E_ADAPTER_MALFORMED"
        E_ADAPTER_BASE + 5 -> "E_ADAPTER_CONSENT_REQUIRED"
        E_ADAPTER_BASE + 6 -> "E_ADAPTER_STALE_GENERATION"
        // Schema (200..201).
        E_SCHEMA_BASE + 0 -> "E_SCHEMA_UNSUPPORTED"
        E_SCHEMA_BASE + 1 -> "E_SCHEMA_INVALID"
        // Object (300..307).
        E_OBJECT_BASE + 0 -> "E_OBJECT_MISSING"
        E_OBJECT_BASE + 1 -> "E_OBJECT_CORRUPT"
        E_OBJECT_BASE + 2 -> "E_OBJECT_STALE"
        E_OBJECT_BASE + 3 -> "E_OBJECT_OVERSIZED"
        E_OBJECT_BASE + 4 -> "E_OBJECT_INVALID"
        E_OBJECT_BASE + 5 -> "E_OBJECT_DIGEST_MISMATCH"
        E_OBJECT_BASE + 6 -> "E_OBJECT_INCOMPLETE_STAGED"
        E_OBJECT_BASE + 7 -> "E_OBJECT_SECRET_EXCLUDED"
        // Transfer (400..405).
        E_TRANSFER_BASE + 0 -> "E_TRANSFER_TIMEOUT"
        E_TRANSFER_BASE + 1 -> "E_TRANSFER_CANCELLED"
        E_TRANSFER_BASE + 2 -> "E_TRANSFER_DIGEST_MISMATCH"
        E_TRANSFER_BASE + 3 -> "E_TRANSFER_CONFLICTING_CHUNK"
        E_TRANSFER_BASE + 4 -> "E_TRANSFER_QUOTA"
        E_TRANSFER_BASE + 5 -> "E_TRANSFER_RESUME_FAILURE"
        // Budget (500..504).
        E_BUDGET_BASE + 0 -> "E_BUDGET_NETWORK"
        E_BUDGET_BASE + 1 -> "E_BUDGET_CPU"
        E_BUDGET_BASE + 2 -> "E_BUDGET_MEMORY"
        E_BUDGET_BASE + 3 -> "E_BUDGET_STORAGE"
        E_BUDGET_BASE + 4 -> "E_BUDGET_TIME"
        // Action (600..603).
        E_ACTION_BASE + 0 -> "E_ACTION_UNSUPPORTED"
        E_ACTION_BASE + 1 -> "E_ACTION_DEPENDENCY_FAILURE"
        E_ACTION_BASE + 2 -> "E_ACTION_EXECUTION_FAILURE"
        E_ACTION_BASE + 3 -> "E_ACTION_ORACLE_FAILURE"
        // Auth (700..703).
        E_AUTH_BASE + 0 -> "E_AUTH_PERMISSION"
        E_AUTH_BASE + 1 -> "E_AUTH_WRONG_EPOCH"
        E_AUTH_BASE + 2 -> "E_AUTH_AMBIGUOUS"
        E_AUTH_BASE + 3 -> "E_AUTH_READ_ONLY"
        // Math (800).
        E_MATH_BASE -> "E_MATH"
        // Internal (900..904).
        E_INTERNAL_BASE + 0 -> "E_INTERNAL_INVARIANT"
        E_INTERNAL_BASE + 1 -> "E_INTERNAL_DB_CORRUPT"
        E_INTERNAL_BASE + 2 -> "E_INTERNAL_IO"
        E_INTERNAL_BASE + 3 -> "E_INTERNAL_SERIALIZATION"
        E_INTERNAL_BASE + 4 -> "E_INTERNAL_IDEMPOTENCY"
        // Proto (1000..1004).
        E_PROTO_BASE + 0 -> "E_PROTO_VERSION"
        E_PROTO_BASE + 1 -> "E_PROTO_FRAMING"
        E_PROTO_BASE + 2 -> "E_PROTO_SEQUENCE"
        E_PROTO_BASE + 3 -> "E_PROTO_REPLAY"
        E_PROTO_BASE + 4 -> "E_PROTO_STATE"
        // Platform (1100..1104).
        E_PLATFORM_BASE + 0 -> "E_PLATFORM_PERMISSION"
        E_PLATFORM_BASE + 1 -> "E_PLATFORM_ACTIVATION"
        E_PLATFORM_BASE + 2 -> "E_PLATFORM_BACKGROUND"
        E_PLATFORM_BASE + 3 -> "E_PLATFORM_SECURE_STORAGE"
        E_PLATFORM_BASE + 4 -> "E_PLATFORM_PACKAGE"
        E_PLATFORM_BASE + 5 -> "E_PLATFORM_DISCOVERY" // ABI 1.2 (§18.3 stage 4)
        else -> if (code > 0) "E_UNKNOWN($code)" else "ERR_UNKNOWN($code)"
    }
}

/** The single exception type raised by the wrappers; [code] is the stable ABI code. */
class CoreException(val code: Int, message: String) :
    Exception(if (message.isBlank()) Err.name(code) else "${Err.name(code)}: $message")

/* ---- protocol helpers (carryon.h conventions) ----------------------------- */

/** Fetch the engine thread-local last_error message. MUST be called on the same
 *  thread as the failed call (thread_local storage in errors.rs). */
fun lastErrorMessage(): String {
    val lenBox = longArrayOf(0)
    CarryOnFfi.lastError(null, lenBox) // query: required size in lenBox[0]
    val required = lenBox[0].toInt()
    if (required <= 1) return "" // empty message (just the NUL)
    val buf = ByteArray(required)
    val rc = CarryOnFfi.lastError(buf, lenBox)
    if (rc != Err.OK) return ""
    val written = lenBox[0].toInt().coerceIn(1, required)
    return String(buf, 0, written - 1, Charsets.UTF_8)
}

/** Out-string ABI result: the shim hands back a UTF-8 copy and already freed the
 *  engine buffer. Null means the call failed -> surface the ENGINE's error code
 *  (device-run bug 2026-11-07: this fabricated Err.UNKNOWN (INT_MIN) — the
 *  DemoRunner could not match typed outcomes like the §6.8 E_BUDGET_NETWORK
 *  refusal (500) and the graph+budget run wedged until the watchdog; the shim
 *  now records the real rc thread-locally, read here on the SAME thread). */
internal fun requireOut(bytes: ByteArray?): String =
    bytes?.toString(Charsets.UTF_8)
        ?: throw CoreException(
            CarryOnFfi.lastOutRc().takeIf { it != Err.OK } ?: Err.UNKNOWN,
            "engine call failed: ${lastErrorMessage()}",
        )

/** The *len query protocol (carryon.h): call once with buf=null for the required
 *  size, allocate, call again. Used by cert/key DER, last_error, read_object. */
internal fun sizedBytes(call: (ByteArray?, LongArray) -> Int): ByteArray {
    val lenBox = longArrayOf(0)
    var rc = call(null, lenBox)
    if (rc != Err.ERR_BUFFER_TOO_SMALL) {
        throw CoreException(rc, lastErrorMessage())
    }
    val buf = ByteArray(lenBox[0].toInt())
    rc = call(buf, lenBox)
    if (rc != Err.OK) throw CoreException(rc, lastErrorMessage())
    return buf.copyOf(lenBox[0].toInt().coerceAtMost(buf.size))
}

/* ---- parsed engine JSON ---------------------------------------------------- */

/** Cut reference `{session,number}` — create_cut output, action-call input. */
data class CutRef(val sessionId: String, val number: Long, val json: String) {
    companion object {
        fun parse(json: String): CutRef {
            val o = JSONObject(json)
            return CutRef(o.getString("session"), o.getLong("number"), json)
        }
    }
}

/** import_cut outcome (abi_transfer.rs `write_outcome`). The `partial_budget`
 *  shape is a NORMAL outcome (§6.8 progressive admission): nothing sealed; retry
 *  with a replenished budget. */
sealed class ImportOutcome {
    data class Completed(val mirrorCut: Long) : ImportOutcome()
    data class Suspended(val resumeTokenJson: String) : ImportOutcome()
    data class PartialBudget(
        val remoteSession: String,
        val remoteCut: Long,
        val published: Int,
        val deferred: Int,
        val deferredBytes: Long,
    ) : ImportOutcome()

    companion object {
        fun parse(json: String): ImportOutcome {
            val o = JSONObject(json)
            return when {
                o.optBoolean("completed") -> Completed(o.getLong("mirror_cut"))
                o.optBoolean("suspended") ->
                    Suspended(o.getJSONObject("resume_token").toString())
                o.optBoolean("partial_budget") -> PartialBudget(
                    o.getString("remote_session"),
                    o.getLong("remote_cut"),
                    o.getJSONArray("published").length(),
                    // Device-run bug 2026-11-07: `deferred` is a JSON ARRAY of
                    // object ids in the engine's write_outcome shape
                    // (abi_transfer.rs serializes Vec<String>) — getInt on it
                    // threw and the run died at the first partial admission.
                    o.getJSONArray("deferred").length(),
                    o.getLong("deferred_bytes"),
                )
                else -> throw CoreException(Err.ERR_BAD_JSON, "import outcome: $json")
            }
        }
    }
}

/** Recovery report fields (abi_core.rs carryon_core_recovery_report_json;
 * §19.3). The three list fields carry the offending ids in the JSON ARRAYS
 * (device-run bug 2026-11-07: this used to call getInt on them —
 * "Value [] at interrupted_transfers of type org.json.JSONArray cannot be
 * converted to int" — and aborted the whole run at OPEN; the shape matches
 * the Rust RecoveryReport and the macOS CarryonCore.swift decode).
 * Interrupted ≠ success (§19.3.9). */
data class RecoveryReport(
    val interruptedTransfers: List<String>,
    val hiddenIncompleteObjects: List<String>,
    val ambiguousSessions: List<String>,
    val journalTruncated: Boolean,
    val clean: Boolean,
) {
    companion object {
        fun parse(json: String): RecoveryReport {
            val o = JSONObject(json)
            // JSON arrays of ids (empty = none found; counts are the sizes).
            fun ids(key: String): List<String> {
                val a = o.getJSONArray(key)
                return List(a.length()) { a.getString(it) }
            }
            return RecoveryReport(
                ids("interrupted_transfers"),
                ids("hidden_incomplete_objects"),
                ids("ambiguous_sessions"),
                o.getBoolean("journal_truncated"),
                o.getBoolean("clean"),
            )
        }
    }
}

/** One pinned peer (trust.rs `TrustRecord`). Only public material — never keys. */
data class TrustRecord(
    val deviceName: String,
    val pinHex: String,
    val trustGeneration: Long,
    val revoked: Boolean,
    val pairedUtc: String,
) {
    /** Truncated pin for UI display (§22.3: no full fingerprint spam either). */
    val shortPin: String get() = pinHex.take(16) + "…"
}

/** Both halves of a mutual pairing (`{local_trust, remote_trust}` JSON). */
data class PairingResult(val localTrustJson: String, val remoteTrustJson: String)

data class AbiVersion(val major: Int, val minor: Int) {
    override fun toString(): String = "$major.$minor"
}

/* ---- handle wrappers -------------------------------------------------------- */

/** Opaque engine core (§6 lifecycle owner). Create in a temp/work dir per run. */
class Core private constructor(internal val handle: Long) : Closeable {
    @Volatile private var closed = false

    companion object {
        fun open(dataDir: String): Core {
            val h = CarryOnFfi.coreOpen(dataDir)
            if (h == 0L) {
                throw CoreException(Err.UNKNOWN, "core_open: ${lastErrorMessage()}")
            }
            return Core(h)
        }
    }

    private fun check(rc: Int) {
        if (rc != Err.OK) throw CoreException(rc, lastErrorMessage())
    }

    private inline fun boolOut(call: (BooleanArray) -> Int): Boolean {
        val out = booleanArrayOf(false)
        check(call(out))
        return out[0]
    }

    fun recoveryReportJson(): String = requireOut(CarryOnFfi.coreRecoveryReportJson(handle))
    fun recoveryReport(): RecoveryReport = RecoveryReport.parse(recoveryReportJson())

    /** Progressive transfer knob: small chunks make the two-phase publish visible. */
    fun setChunkSize(bytes: Long) = check(CarryOnFfi.coreSetChunkSize(handle, bytes))

    /** Budget JSON (§6.8, model/budget.rs): total_net_bytes, avg_net_rate,
     *  burst_bytes, total_cpu_ms, cpu_duty, peak_prep_mem, storage_quota,
     *  battery_thermal, elapsed_opportunity_ms, max_nonpreemptible_ms. */
    fun setBudget(budgetJson: String) = check(CarryOnFfi.coreSetBudget(handle, budgetJson))

    /** Foreground-first admission (§20.2): background shrinks the budget. */
    fun setForeground(fg: Boolean) = check(CarryOnFfi.coreSetForeground(handle, fg))

    /** Cooperative suspend for import (§11.2): import_cut returns a resume token. */
    fun requestSuspend() = check(CarryOnFfi.coreRequestSuspend(handle))

    /** Compiled-in adapter selection by id + JSON params (data, never behavior — §3.8). */
    fun registerAdapter(adapterId: String, paramsJson: String): String =
        requireOut(CarryOnFfi.registerAdapter(handle, adapterId, paramsJson))

    fun createSession(createReqJson: String): String =
        requireOut(CarryOnFfi.createSession(handle, createReqJson))

    fun createCut(sessionId: String): CutRef =
        CutRef.parse(requireOut(CarryOnFfi.createCut(handle, sessionId)))

    fun listAvailableActions(cutJson: String, classesJson: String = "[]"): String =
        requireOut(CarryOnFfi.listAvailableActions(handle, cutJson, classesJson))

    fun executeAction(cutJson: String, actionReqJson: String): String =
        requireOut(CarryOnFfi.executeAction(handle, cutJson, actionReqJson))

    /** Continue an imported (mirror) session: the adapter id is DATA naming a
     *  compiled-in adapter (§3.8/§7.4), here `imported` per AGENTS.md. */
    fun executeActionAs(
        cutJson: String, actionReqJson: String, adapterId: String
    ): String = requireOut(CarryOnFfi.executeActionAs(handle, cutJson, actionReqJson, adapterId))

    fun hasObjectHex(contentHash: String): Boolean =
        boolOut { out -> CarryOnFfi.hasObjectHex(handle, contentHash, out) }

    /** Object bytes via the *len protocol; content_hash verified by the core
     *  before publish (CORE-004 two-phase invariant). */
    fun readObjectBytes(contentHash: String): ByteArray =
        sizedBytes { buf, len -> CarryOnFfi.readObject(handle, contentHash, buf, len) }

    /** L4 single-writer gate (§21.2): authoritative writes only when true. */
    fun mayMutate(sessionId: String): Boolean =
        boolOut { out -> CarryOnFfi.mayMutate(handle, sessionId, out) }

    /** §21.3 recovery: fresh epoch + conflict-risk record. */
    fun recoverAuthority(sessionId: String): Long {
        val out = longArrayOf(0)
        check(CarryOnFfi.recoverAuthority(handle, sessionId, out))
        return out[0]
    }

    fun exportEvidence(sessionId: String): String =
        requireOut(CarryOnFfi.exportEvidence(handle, sessionId))

    /** Evidence bundle + measured continuation metrics (the carryon_l3.c shape). */
    fun exportEvidenceWith(sessionId: String, extraMetricsJson: String): String =
        requireOut(CarryOnFfi.exportEvidenceWith(handle, sessionId, extraMetricsJson))

    fun verifyEvidence(bundlePath: String): String =
        requireOut(CarryOnFfi.verifyEvidence(handle, bundlePath))

    override fun close() {
        if (!closed) {
            closed = true
            CarryOnFfi.coreFree(handle)
        }
    }
}

/** Device identity. Public surface = pin hex ONLY (§22.3, EVD-006). */
class Identity private constructor(internal val handle: Long) : Closeable {
    @Volatile private var closed = false

    companion object {
        fun generate(name: String): Identity {
            val h = CarryOnFfi.identityGenerate(name)
            if (h == 0L) {
                throw CoreException(Err.UNKNOWN, "identity_generate: ${lastErrorMessage()}")
            }
            return Identity(h)
        }
    }

    /** SHA-256 pin hex of the certificate DER — the pairing fingerprint. */
    fun pinHex(): String = requireOut(CarryOnFfi.identityPinHex(handle))

    /** Truncated fingerprint for display (§22.3). */
    fun shortPin(): String = pinHex().take(16) + "…"

    override fun close() {
        if (!closed) {
            closed = true
            CarryOnFfi.identityFree(handle)
        }
    }
}

/** Pinned-peer trust store (§18.4/§18.5). Plain serde data — persist freely. */
class Trust private constructor(internal val handle: Long) : Closeable {
    @Volatile private var closed = false

    companion object {
        /** TrustStore JSON: `{"peers":{"<pin_hex>":{device_name,pin_hex,
         *  trust_generation,revoked,paired_utc}}}` (carryon-net/src/trust.rs). */
        fun fromJson(json: String): Trust {
            val h = CarryOnFfi.trustFromJson(json)
            if (h == 0L) {
                throw CoreException(Err.UNKNOWN, "trust_from_json: ${lastErrorMessage()}")
            }
            return Trust(h)
        }

        fun empty(): Trust = fromJson("{\"peers\":{}}")
    }

    private fun check(rc: Int) {
        if (rc != Err.OK) throw CoreException(rc, lastErrorMessage())
    }

    fun toJson(): String = requireOut(CarryOnFfi.trustToJson(handle))

    /** Pair a peer by pin (§18.4.6-7). */
    fun pair(deviceName: String, pinHex: String, utc: String) =
        check(CarryOnFfi.trustPair(handle, deviceName, pinHex, utc))

    fun isTrusted(pinHex: String): Boolean {
        val out = booleanArrayOf(false)
        check(CarryOnFfi.trustIsTrusted(handle, pinHex, out))
        return out[0]
    }

    /** §18.4.9 revocation: bumps the trust generation; reconnect fails closed. */
    fun revoke(pinHex: String): Boolean {
        val out = booleanArrayOf(false)
        check(CarryOnFfi.trustRevoke(handle, pinHex, out))
        return out[0]
    }

    fun records(): List<TrustRecord> {
        val peers = JSONObject(toJson()).optJSONObject("peers") ?: JSONObject()
        return peers.keys().asSequence().map { pin ->
            val r = peers.getJSONObject(pin)
            TrustRecord(
                deviceName = r.getString("device_name"),
                pinHex = r.getString("pin_hex"),
                trustGeneration = r.getLong("trust_generation"),
                revoked = r.getBoolean("revoked"),
                pairedUtc = r.getString("paired_utc"),
            )
        }.toList()
    }

    override fun close() {
        if (!closed) {
            closed = true
            CarryOnFfi.trustFree(handle)
        }
    }
}

/** TLS 1.3 listener (§17). Bind "127.0.0.1:0" for the LOCAL demo loopback. */
class Listener private constructor(internal val handle: Long) : Closeable {
    @Volatile private var closed = false

    companion object {
        fun bind(addr: String): Listener {
            val h = CarryOnFfi.listenerBind(addr)
            if (h == 0L) {
                throw CoreException(Err.UNKNOWN, "listener_bind($addr): ${lastErrorMessage()}")
            }
            return Listener(h)
        }
    }

    fun addr(): String = requireOut(CarryOnFfi.listenerAddr(handle))

    override fun close() {
        if (!closed) {
            closed = true
            CarryOnFfi.listenerFree(handle)
        }
    }
}

/** An established mutual-pinned TLS session (§16/§17). Counters are MEASURED. */
class Session private constructor(internal val handle: Long) : Closeable {
    @Volatile private var closed = false

    companion object {
        fun connect(addr: String, identity: Identity, trust: Trust): Session {
            val h = CarryOnFfi.sessionConnect(addr, identity.handle, trust.handle)
            if (h == 0L) {
                throw CoreException(Err.UNKNOWN, "session_connect($addr): ${lastErrorMessage()}")
            }
            return Session(h)
        }

        fun accept(listener: Listener, identity: Identity, trust: Trust): Session {
            val h = CarryOnFfi.sessionAccept(listener.handle, identity.handle, trust.handle)
            if (h == 0L) {
                throw CoreException(Err.UNKNOWN, "session_accept: ${lastErrorMessage()}")
            }
            return Session(h)
        }
    }

    private fun check(rc: Int) {
        if (rc != Err.OK) throw CoreException(rc, lastErrorMessage())
    }

    fun clientNegotiate(featuresJson: String = "[]"): String =
        requireOut(CarryOnFfi.sessionClientNegotiate(handle, featuresJson))

    fun serverNegotiate(featuresJson: String = "[]"): String =
        requireOut(CarryOnFfi.sessionServerNegotiate(handle, featuresJson))

    /** -> (sent, recv) wire counters. Sampled ONLY between completed wire
     *  operations: a CarryonSession is NOT thread-safe (carryon.h — the handle
     *  must not be aliased during a blocking call); never poll it while
     *  import/serve/authority is in flight on the same session. */
    fun bytes(): Pair<Long, Long> {
        val out = longArrayOf(0, 0)
        check(CarryOnFfi.sessionBytes(handle, out))
        return out[0] to out[1]
    }

    override fun close() {
        if (!closed) {
            closed = true
            CarryOnFfi.sessionFree(handle)
        }
    }
}

/* ---- free functions ---------------------------------------------------------- */

/** Check the engine ABI at load: expect 1.2 (append-only within major 1;
 * 1.2 added the discovery contract entries, §18.3 stage 4). */
fun abiVersion(): AbiVersion {
    val v = CarryOnFfi.abiVersion()
        ?: throw CoreException(Err.UNKNOWN, "abi_version returned null")
    return AbiVersion(v[0], v[1])
}

/** Mutual pin pairing (§18.4): each side receives its half of the trust store. */
fun pairDevices(a: Identity, b: Identity, utc: String): PairingResult {
    val json = requireOut(CarryOnFfi.pairDevices(a.handle, b.handle, utc))
    val o = JSONObject(json)
    return PairingResult(
        localTrustJson = o.getJSONObject("local_trust").toString(),
        remoteTrustJson = o.getJSONObject("remote_trust").toString(),
    )
}

/** Deterministic mirror session id for an imported session (AGENTS.md: adapter id
 *  `imported`; use execute_action_as to continue). */
fun mirrorSessionId(remoteSession: String): String =
    requireOut(CarryOnFfi.mirrorSessionId(remoteSession))

/* ---- transfer + authority (blocking calls; run on Dispatchers.IO) ---------- */

/** serve_cut — BLOCKING until the peer's import finishes/suspends/aborts.
 *  The wire stays open afterwards: serve_authority_transfer may follow on the
 *  SAME session (the carryon_l3.c flow). */
fun Core.serveCut(session: Session) {
    val rc = CarryOnFfi.serveCut(handle, session.handle)
    if (rc != Err.OK) throw CoreException(rc, lastErrorMessage())
}

/** import_cut — pull → re-verify → publish → seal mirror cut (CORE-004). */
fun Core.importCut(
    session: Session, remoteSession: String, remoteCut: Long
): ImportOutcome = ImportOutcome.parse(
    requireOut(CarryOnFfi.importCut(handle, session.handle, remoteSession, remoteCut))
)

/** resume_import (§11.2) — on a FRESH connection (the suspended wire is closed;
 *  see crates/carryon-ffi/tests/round_trip.rs suspend/resume flow). */
fun Core.resumeImport(session: Session, resumeTokenJson: String): ImportOutcome =
    ImportOutcome.parse(
        requireOut(CarryOnFfi.resumeImport(handle, session.handle, resumeTokenJson))
    )

/** Source side of the L4 transfer (§21.2): propose → await accept → commit
 *  relinquishment → drop to read-only. BLOCKING (peer's request drives it). */
fun Core.serveAuthorityTransfer(session: Session, sessionId: String, cutNumber: Long): String =
    requireOut(CarryOnFfi.serveAuthorityTransfer(handle, session.handle, sessionId, cutNumber))

/** Destination side of the L4 transfer: accept → pending epoch → finalize on
 *  source receipt → owner. Receipt carries the new epoch. */
fun Core.requestAuthorityTransfer(
    session: Session, mirrorSessionId: String, adapterId: String
): String = requireOut(
    CarryOnFfi.requestAuthorityTransfer(handle, session.handle, mirrorSessionId, adapterId)
)

/* ---- discovery contract (spec §18.3, stage 4; ABI 1.2) --------------------- */

/** The engine-validated fields of a §18.3 TXT record. Mirrors the engine's
 * canonical JSON (protocol_major, instance_id, pairing_required, device_class,
 * connection_hint). */
data class DiscoveryTxt(
    val protocolMajor: Int,
    val instanceId: String,
    val pairingRequired: Boolean,
    val deviceClass: String,
    val connectionHint: String,
)

/**
 * The §18.3 TXT contract as owned by the ENGINE. Android's NSD responder does
 * the actual registering/browsing (app layer), but TXT construction and every
 * received record go through here: [encode] can only ever emit the five
 * minimal keys, and [validate] fails CLOSED on a malformed record (throws —
 * the caller MUST drop the peer, never repair it); foreign keys are returned
 * as ignored diagnostics, never interpreted.
 */
object DiscoveryContract {
    /** Field values -> canonical wire pairs "[[key,value],…]" (engine-validated;
     * throws on a contract violation, e.g. a non-coarse device class). */
    fun encode(txt: DiscoveryTxt): String {
        val fields = JSONObject().apply {
            put("protocol_major", txt.protocolMajor)
            put("instance_id", txt.instanceId)
            put("pairing_required", txt.pairingRequired)
            put("device_class", txt.deviceClass)
            put("connection_hint", txt.connectionHint)
        }
        return requireOut(CarryOnFfi.discoveryTxtEncode(fields.toString()))
    }

    /**
     * Validate RECEIVED pairs (ALL of them, foreign keys included — the engine
     * reports what it ignored). Returns the validated fields + ignored keys.
     * @throws CoreException code E_PLATFORM_DISCOVERY (1105) on ANY missing or
     * malformed required key — the peer MUST be dropped (fail closed, §18.3).
     */
    fun validate(pairs: List<Pair<String, String>>): Pair<DiscoveryTxt, List<String>> {
        val arr = JSONArray().apply {
            pairs.forEach { (k, v) -> put(JSONArray().put(k).put(v)) }
        }
        val reply = JSONObject(requireOut(CarryOnFfi.discoveryTxtValidate(arr.toString())))
        val txt = reply.getJSONObject("txt")
        val ignored = reply.optJSONArray("ignored_keys")
            ?.let { a -> List(a.length()) { a.getString(it) } }
            ?: emptyList()
        return DiscoveryTxt(
            protocolMajor = txt.getInt("protocol_major"),
            instanceId = txt.getString("instance_id"),
            pairingRequired = txt.getBoolean("pairing_required"),
            deviceClass = txt.getString("device_class"),
            connectionHint = txt.optString("connection_hint"),
        ) to ignored
    }
}
