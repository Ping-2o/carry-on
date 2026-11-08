/*
 * HandoffFlow.kt — the two sides of the L0/L1 file handoff (stage 3, manual-IP
 * mode), driving the REAL engine through the C ABI:
 *
 *   serveFile   — the SOURCE (mac side; used by the app-parity smoke binary on
 *                 the mac and by the on-device loopback instrumented test): a
 *                 real file captured by the compiled-in file adapter (§10.7
 *                 content-hash identity), sealed cut, connect to the
 *                 hand-entered receiver, descriptor in the Hello features,
 *                 serve_cut until ImportComplete.
 *
 *   receiveFile — the DESTINATION (the "Receive" screen's engine): accept the
 *                 pinned source, read the descriptor from the negotiation,
 *                 import_cut, then the CORE-004 app-level gate: has_object +
 *                 measured sha256 == declared, or REFUSE (fail closed, no
 *                 open). Then the honest L1: the engine's activation-only
 *                 `document.open` record (§6.5 OracleKind::None) + the real
 *                 OS ACTION_VIEW intent with the verified bytes.
 *
 * Descriptor channel (§3.8-safe): the Hello/Welcome features carry DATA ONLY
 * (session, cut, digest, name, mime, evidence class); unknown features are
 * ignored, never interpreted; a malformed descriptor refuses closed.
 *
 * Threading + lifetime (DemoRunner rules): every blocking engine call runs on
 * Dispatchers.IO; a blocked JNI call cannot be interrupted — callers that give
 * up on an in-flight accept leak the listener deliberately (freeing under a
 * blocked call is use-after-free); a fresh run arms a FRESH listener + core.
 * Counters are sampled only BETWEEN completed wire operations (a
 * CarryonSession is NOT thread-safe).
 *
 * Labels (§2 law): the evidence class ("local" | "physical") is derived from
 * the address the source dials and carried in the descriptor + bundles. No
 * platform is "supported" (§30 unrun) either way.
 */
package com.carryon.app.handoff

import com.carryon.app.ffi.Core
import com.carryon.app.ffi.CoreException
import com.carryon.app.ffi.Err
import com.carryon.app.ffi.Identity
import com.carryon.app.ffi.ImportOutcome
import com.carryon.app.ffi.Listener
import com.carryon.app.ffi.Session
import com.carryon.app.ffi.Trust
import com.carryon.app.ffi.abiVersion
import com.carryon.app.ffi.importCut
import com.carryon.app.ffi.mirrorSessionId
import com.carryon.app.ffi.pairDevices
import com.carryon.app.ffi.serveCut
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.time.Instant
import java.time.format.DateTimeFormatter

/** The ladder of the L0/L1 handoff (spec ladder semantics: L0 = files, L1 =
 *  launcher activation — the same order as the onboarding ladder). */
enum class HandoffLadderStep(val label: String) {
    L0_CARRY("L0 ─ file carried + verified"),
    L1_OPEN("L1 ─ opened in a real app"),
}

enum class HandoffOutcome { PENDING, RUNNING, OK, SKIPPED, FAILED }

/** Events the flows emit; the UI (and tests) consume them live. */
sealed class HandoffEvent {
    data class Console(val line: String) : HandoffEvent()
    data class Ladder(val step: HandoffLadderStep, val outcome: HandoffOutcome) : HandoffEvent()
    /** Exact §16.2 session state names (never the bare word "Ready"). */
    data class State(val state: String) : HandoffEvent()
    data class Counters(val sent: Long, val recv: Long) : HandoffEvent()
    /** The receiver's FULL 64-hex pin as soon as it exists (public
     *  fingerprint; EVD-006 excludes only private keys). */
    data class ReceiverPin(val pinHex: String) : HandoffEvent()
    data class Finished(val result: ReceiveResult) : HandoffEvent()
    data class Failed(val why: String) : HandoffEvent()
}

/** What one receive produced (both sides' facts the UI + evidence need). */
data class ReceiveResult(
    val ok: Boolean,
    val refused: Boolean,          // CORE-004 gate refused (integrity negative)
    val mirrorSession: String,
    val mirrorCut: Long,
    val declaredHash: String,
    val verifiedBytes: File?,      // the byte-for-byte reconstruction in app cache
    val contentUri: String?,       // the MediaStore URI handed to ACTION_VIEW
    val resolvedActivity: String?, // what the OS chose for ACTION_VIEW (may be null)
    val bundleJson: String?,
    val verifyOk: Boolean,
    val bytesSent: Long,
    val bytesRecv: Long,
    val ttaMs: Double,
    val evidenceClass: String,
    val descriptor: HandoffDescriptor,
) {
    companion object {
        /** The receive screen's honest footer — distinct from the demo's
         *  always-LOCAL footer (a handoff can be PHYSICAL). */
        fun disclosure(evidenceClass: String, sourceAddr: String): String =
            if (evidenceClass == "physical") {
                "DISCLOSURE: PHYSICAL cross-device handoff over LAN TLS 1.3 (two machines, two NICs;\n" +
                    "source descriptor: $sourceAddr). NOT a supported platform (spec §2/§30)."
            } else {
                "DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and\n" +
                    "engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30)."
            }
    }
}

object HandoffFlow {

    /* ---- SOURCE side ------------------------------------------------------- */

    /**
     * Serve one real file to the receiver at `addr` (hand-entered IPv4:port).
     * `destPin` is the receiver's pairing pin. `identity` may be pre-generated
     * so the caller can hand its pin to the receiver BEFORE connecting (the
     * manual-mode human channel). Returns the source session id.
     */
    suspend fun serveFile(
        dataDir: File,
        file: File,
        addr: String,
        destPin: String,
        identity: Identity? = null,
        tamper: Boolean = false,
        emitter: (HandoffEvent) -> Unit = {},
    ): String = withContext(Dispatchers.IO) {
        val bytes = file.readBytes()
        require(bytes.isNotEmpty()) { "refusing to carry an empty file" }
        require(bytes.size <= 16 * 1024 * 1024) {
            "file exceeds the file adapter's 16 MiB object limit (§26.5.3)"
        }
        require(HandoffValidation.isValidReceiverAddr(addr)) {
            "receiver address must be IPv4:port (e.g. 192.168.1.20:48961)"
        }
        require(HandoffValidation.isValidPinHex(destPin)) {
            "receiver pin must be the 64-hex fingerprint shown on its Receive screen"
        }
        val trueHash = HandoffValidation.sha256Hex(bytes)
        // Integrity negative: declare the digest of TAMPERED bytes — the
        // receiver's CORE-004 gate must REFUSE (the visible-failure counterpart
        // of the demo's §6.8 budget showcase).
        val declaredHash = if (tamper) HandoffValidation.sha256Hex(bytes + 0x00) else trueHash
        val evidenceClass = HandoffValidation.evidenceClass(dialing = addr)

        emitter(HandoffEvent.Console("== Carry-On serve file — L0/L1 (manual IP, pre-discovery) =="))
        emitter(HandoffEvent.Console("file:  ${file.absolutePath}  (${bytes.size} bytes)"))
        emitter(HandoffEvent.Console("sha256:  $trueHash  (declared)"))
        emitter(HandoffEvent.Console("evidence_class:  $evidenceClass"))
        if (tamper) {
            emitter(HandoffEvent.Console("integrity_test:  NEGATIVE — declared hash tampered; receiver MUST refuse (CORE-004)"))
        }

        Core.open(dataDir.absolutePath).use { core ->
            val id = identity ?: Identity.generate("carryon-src")
            id.use {
                val srcPin = id.pinHex()
                emitter(HandoffEvent.ReceiverPin(srcPin))
                emitter(HandoffEvent.Console("source_pin:  ${srcPin.take(16)}…  (full pin on the source console)"))
                val trust = Trust.empty()
                trust.use {
                    trust.pair("receiver", destPin, utcNow())
                    core.registerAdapter("org.carryon.file", "{\"path\":\"${jsonEscape(file.absolutePath)}\"}")
                    emitter(HandoffEvent.Console("adapter:  org.carryon.file  (compiled-in, §10.7 L1 content-hash identity)"))
                    val session = core.createSession(
                        JSONObject().apply {
                            put("adapter_id", "org.carryon.file")
                            put("title", "file handoff: ${file.name}")
                            put("privacy", "personal")
                            put("authority_mode", "read_only_replica")
                        }.toString()
                    )
                    val cut = core.createCut(session)
                    emitter(HandoffEvent.State("CutSealed"))
                    emitter(HandoffEvent.Console("session:  $session"))
                    emitter(HandoffEvent.Console("cut:  sealed number=${cut.number} (two-phase publish, CORE-004)"))

                    emitter(HandoffEvent.Console("connect:  $addr  (TLS 1.3, mutual certificate pinning §18.5)"))
                    val descriptor = HandoffDescriptor(
                        session = session, cut = cut.number, contentHash = declaredHash,
                        fileName = file.name,
                        mimeType = HandoffValidation.mimeForExtension(file.extension),
                        evidenceClass = evidenceClass,
                    )
                    Session.connect(addr, id, trust).use { wire ->
                        val theirs = wire.clientNegotiate(
                            JSONArray(descriptor.features()).toString()
                        )
                        emitter(HandoffEvent.Console("negotiate:  descriptor sent (data only, §3.8); theirs: $theirs"))
                        emitter(HandoffEvent.State("Importing"))
                        core.serveCut(wire)
                        val (sent, recv) = wire.bytes() // between completed ops only
                        emitter(HandoffEvent.Counters(sent, recv))
                        emitter(HandoffEvent.Console("serve:  cut served — destination reported ImportComplete"))
                        emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.OK))
                        emitter(HandoffEvent.State("ActionReady"))
                        emitter(HandoffEvent.Console("L1:  launcher activation runs on the receiver (engine document.open + OS ACTION_VIEW)"))
                        session
                    }
                }
            }
        }
    }

    /* ---- DESTINATION side --------------------------------------------------- */

    /**
     * One armed receiver: the engine core, identity, and bound listener the
     * UI displays (address + full pin) BEFORE the source is started. A fresh
     * arm per handoff; close() on abandon (never while a wire op runs).
     */
    class Armed internal constructor(
        val core: Core,
        val identity: Identity,
        val listener: Listener,
        /** FULL 64-hex pin of this receiver (public fingerprint). */
        val pinHex: String,
        /** The bound wire address (e.g. "0.0.0.0:54321"). */
        val wireAddr: String,
        /** The display address for the source: "<site-local-ipv4>:<port>". */
        val displayAddr: String,
    ) : java.io.Closeable {
        /** Truncated pin for §22.3-style display. */
        val shortPin: String get() = pinHex.take(16) + "…"

        override fun close() {
            runCatching { listener.close() }
            runCatching { identity.close() }
            runCatching { core.close() }
        }
    }

    /**
     * Arm a receiver: open the core, generate a per-handoff identity, bind the
     * listener, display. Cheap and cancellable — nothing blocks here.
     */
    suspend fun arm(workDir: File): Armed = withContext(Dispatchers.IO) {
        workDir.mkdirs()
        val core = Core.open(workDir.resolve("dest").absolutePath)
        val identity = Identity.generate("carryon-receiver")
        val listener = Listener.bind("0.0.0.0:0")
        val pin = identity.pinHex()
        val wireAddr = listener.addr()
        val port = wireAddr.substringAfterLast(':')
        Armed(
            core = core, identity = identity, listener = listener, pinHex = pin,
            wireAddr = wireAddr, displayAddr = "${siteLocalIpv4() ?: "<no-wifi-ip>"}:$port",
        )
    }

    /**
     * The full receive: accept (BLOCKING until the source connects), negotiate
     * (capture the descriptor), import, CORE-004 gate, reconstruction, the
     * engine's document.open record, the OS ACTION_VIEW, and the evidence
     * bundle. `launcher` performs the last step (platform side of L1).
     */
    suspend fun receive(
        armed: Armed,
        sourcePin: String,
        chunkSize: Long,
        workDir: File,
        launcher: (bytes: ByteArray, fileName: String, mime: String) -> Pair<String, String?>,
        emitter: (HandoffEvent) -> Unit = {},
    ): ReceiveResult = withContext(Dispatchers.IO) {
        require(HandoffValidation.isValidPinHex(sourcePin)) {
            "source pin must be the 64-hex fingerprint shown on the source screen"
        }
        val core = armed.core
        val emitterLog: (String, String) -> Unit = { k, v -> emitter(HandoffEvent.Console("$k:  $v")) }

        emitter(HandoffEvent.Console("== Carry-On Android receive — L0/L1 (manual IP, pre-discovery) =="))
        val abi = abiVersion()
        emitterLog("abi", "engine $abi (expect 1.2)")
        emitterLog("core", workDir.resolve("dest").absolutePath)
        // A file-adapter instance just satisfies registration on the receiver —
        // the import verifies bytes, and the store holds the exact content
        // (the reference shell registers a placeholder the same way).
        val placeholder = workDir.resolve("placeholder.txt").apply { writeText("receiver store placeholder") }
        core.registerAdapter("org.carryon.file", "{\"path\":\"${jsonEscape(placeholder.absolutePath)}\"}")
        emitterLog("adapter", "org.carryon.file registered (placeholder path; import verifies bytes, §10.7)")
        emitter(HandoffEvent.ReceiverPin(armed.pinHex))
        emitterLog("pin", "${armed.shortPin}  (full pin shown above; public fingerprint)")

        val trust = Trust.empty()
        trust.use {
            trust.pair("source", sourcePin, utcNow())
            emitterLog("trust", "source pin ${sourcePin.take(16)}… paired (mutual pin, §18.4)")
            emitterLog("listener", "bound ${armed.wireAddr}  (display ${armed.displayAddr}; TLS 1.3, mutual pinning §18.5)")

            emitter(HandoffEvent.State("Importing"))
            emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.RUNNING))
            val tImport0 = System.nanoTime()
            val descriptor: HandoffDescriptor
            var sent = 0L
            var recv = 0L
            Session.accept(armed.listener, armed.identity, trust).use { wire ->
                // The descriptor arrives in the source's Hello features (data
                // only, §3.8). Parse BEFORE anything else; refuse closed.
                descriptor = HandoffDescriptor.parse(wire.serverNegotiate("[]").let { jsonList(it) })
                    ?: throw CoreException(Err.ERR_BAD_JSON, "handoff descriptor missing/malformed — refusing (§3.8)")
                emitterLog("negotiate", "descriptor received (data only, §3.8)")
                emitterLog("descriptor", "session=${descriptor.session} cut=${descriptor.cut} " +
                    "hash=${descriptor.contentHash.take(16)}… name=${descriptor.fileName} " +
                    "mime=${descriptor.mimeType} evid=${descriptor.evidenceClass}")

                // Chunking is DESTINATION-controlled (§18.6): clamp the UI knob.
                if (chunkSize > 0) {
                    val clamped = HandoffValidation.clampChunkSize(chunkSize)
                    core.setChunkSize(clamped)
                    emitterLog("chunk_size", "$clamped B (destination-controlled, §18.6)")
                }

                val outcome = core.importCut(wire, descriptor.session, descriptor.cut)
                val (s, r) = wire.bytes() // between completed ops only
                sent = s
                recv = r
                emitter(HandoffEvent.Counters(sent, recv))
                val ttaMs = (System.nanoTime() - tImport0) / 1e6
                val mirrorCut = when (outcome) {
                    is ImportOutcome.Completed -> {
                        emitterLog("import", "completed (mirror_cut=${outcome.mirrorCut})")
                        outcome.mirrorCut
                    }
                    else -> throw CoreException(Err.E_TRANSFER_BASE + 5, "import did not complete: $outcome")
                }
                emitterLog("bytes", "sent=$sent recv=$recv (MEASURED, §5.6)")
                emitterLog("time_to_action_ready", "%.1f ms".format(ttaMs))

                // ---- CORE-004 app-level gate: declared == measured, or REFUSE.
                // The engine guarantees the store (stage → verify → atomic
                // publish); this gate guards the app's reconstruction path and
                // refuses closed. NO success on a mismatched object, ever.
                val mirror = mirrorSessionId(descriptor.session)
                emitterLog("mirror_session", mirror)
                val present = core.hasObjectHex(descriptor.contentHash)
                if (!present) {
                    emitterLog("gate", "declared object ABSENT in local store → REFUSED (fail closed, CORE-004)")
                    emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.FAILED))
                    emitter(HandoffEvent.State("Failed"))
                    throw CoreException(Err.E_OBJECT_BASE + 5, "declared content hash ${descriptor.contentHash.take(16)}… not present after import — REFUSED (CORE-004)")
                }
                emitterLog("L0", "object ${descriptor.contentHash.take(16)}… present in local store (source-independent)")
                val bytes = core.readObjectBytes(descriptor.contentHash)
                if (!IntegrityGate.verify(bytes, descriptor.contentHash)) {
                    emitterLog("gate", "sha256 measured != declared → REFUSED (fail closed, CORE-004)")
                    emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.FAILED))
                    emitter(HandoffEvent.State("Failed"))
                    throw CoreException(Err.E_OBJECT_BASE + 5, "measured sha256 != declared content hash — REFUSED (CORE-004)")
                }
                emitterLog("gate", "sha256 measured == declared → VERIFIED (byte-for-byte)")

                // Reconstruct the real file for launch (the store object IS the
                // exact bytes; the copy is the ACTION_VIEW target).
                val out = File(workDir, descriptor.fileName)
                out.writeBytes(bytes)
                emitterLog("L0", "file carried + verified → ${out.name} (${bytes.size} bytes)")
                emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.OK))

                // ---- honest L1: the real OS ACTION_VIEW first (PLAT-003, the
                // shell's flow), then the engine's activation-only record with
                // the REAL content uri (§6.5 OracleKind::None — L1 claims
                // identity + launch, never unsaved state).
                emitter(HandoffEvent.State("ActionReady"))
                val (uri, resolved) = launcher(bytes, descriptor.fileName, descriptor.mimeType)
                val cutRef = JSONObject().apply {
                    put("session", mirror)
                    put("number", mirrorCut)
                }
                val open = core.executeActionAs(
                    cutRef.toString(),
                    JSONObject().apply {
                        put("class", "document.open")
                        put("params", JSONObject().put("uri", uri))
                    }.toString(),
                    "org.carryon.file",
                )
                val openDetail = JSONObject(open)
                emitterLog("document.open", "engine activation record ok=${openDetail.optBoolean("activated", false)} (activation-only, §6.5)")

                if (resolved == null) {
                    emitterLog("L1", "no ACTION_VIEW handler resolved for ${descriptor.mimeType} — carried + verified, NOT opened")
                    emitter(HandoffEvent.Ladder(HandoffLadderStep.L1_OPEN, HandoffOutcome.FAILED))
                } else {
                    emitterLog("L1", "ACTION_VIEW ${descriptor.mimeType} → $resolved")
                    emitter(HandoffEvent.Ladder(HandoffLadderStep.L1_OPEN, HandoffOutcome.OK))
                }

                // ---- evidence (destination session, l3-style measured metrics)
                val extra = JSONObject().apply {
                    put("level", "L0-L1-file-handoff")
                    put("mirror_session_id", mirror)
                    put("mirror_cut", mirrorCut)
                    put("source_session_id", descriptor.session)
                    put("file_name", descriptor.fileName)
                    put("file_bytes", bytes.size)
                    put("content_sha256_declared", descriptor.contentHash)
                    put("content_sha256_verified", true)
                    put("integrity_refused", false)
                    put("action_view_resolved", resolved != null)
                    put("resolved_activity", resolved ?: "")
                    put("content_uri", uri)
                    put("evidence_class", descriptor.evidenceClass)
                    put("bytes_transferred_sent", sent)
                    put("bytes_transferred_recv", recv)
                    put("time_to_action_ready_ms", ttaMs)
                    put("adapter", "org.carryon.file")
                    put("authority_transfer", "none — L0/L1 is activation-only (honest: no unsaved app state claimed, §10.7)")
                }
                val bundle = core.exportEvidenceWith(mirror, extra.toString())
                val bundleFile = File(workDir, "evidence.json").apply { writeText(bundle) }
                emitterLog("evidence", "exported ${bundle.length} B -> ${bundleFile.name}")
                val report = core.verifyEvidence(bundleFile.absolutePath)
                val verifyOk = JSONObject(report).optBoolean("ok", false)
                emitterLog("verify_evidence", report.take(160))
                emitter(HandoffEvent.Console(ReceiveResult.disclosure(descriptor.evidenceClass, armed.displayAddr)))

                ReceiveResult(
                    ok = verifyOk, refused = false, mirrorSession = mirror,
                    mirrorCut = mirrorCut, declaredHash = descriptor.contentHash,
                    verifiedBytes = out, contentUri = uri, resolvedActivity = resolved,
                    bundleJson = bundle, verifyOk = verifyOk, bytesSent = sent,
                    bytesRecv = recv, ttaMs = ttaMs,
                    evidenceClass = descriptor.evidenceClass, descriptor = descriptor,
                )
            }
        }
    }

    /* ---- helpers ------------------------------------------------------------ */

    fun utcNow(): String =
        DateTimeFormatter.ISO_INSTANT.format(Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS))

    private fun jsonEscape(s: String): String =
        s.replace("\\", "\\\\").replace("\"", "\\\"")

    /** Parse the engine's features JSON array ("[\"a\",\"b\"]") into a list. */
    fun jsonList(featuresJson: String): List<String> {
        val arr = JSONArray(featuresJson)
        return List(arr.length()) { arr.getString(it) }
    }

    /** This device's site-local IPv4 (no permissions; plain interface enum). */
    fun siteLocalIpv4(): String? =
        java.net.NetworkInterface.getNetworkInterfaces().asSequence()
            .filter { it.isUp && !it.isLoopback }
            .flatMap { it.inetAddresses.asSequence() }
            .firstOrNull { it is java.net.Inet4Address && it.isSiteLocalAddress }
            ?.hostAddress
}
