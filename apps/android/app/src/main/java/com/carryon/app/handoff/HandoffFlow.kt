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

import dev.carryon.kit.Core
import dev.carryon.kit.CoreException
import dev.carryon.kit.Err
import dev.carryon.kit.Identity
import dev.carryon.kit.ImportOutcome
import dev.carryon.kit.Listener
import dev.carryon.kit.Session
import dev.carryon.kit.Trust
import dev.carryon.kit.abiVersion
import dev.carryon.kit.importCut
import dev.carryon.kit.mirrorSessionId
import dev.carryon.kit.pairDevices
import dev.carryon.kit.resumeImport
import dev.carryon.kit.serveCut
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
    /** Wire RECV before the FIRST action completed (§5.6 measured; sampled
     *  only between completed wire ops — the open adds no wire traffic). */
    val bytesBeforeFirstAction: Long = 0,
    /** How many accept/import legs the transfer used (§11.2: >1 means a
     *  cooperative suspend was resumed on a fresh connection). */
    val legsUsed: Int = 1,
    /** The trust came from the PAIRED durable store (no pin typed). */
    val pairedTrust: Boolean = false,
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
        serveLegs: Int = 1,
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
                    // Stage 6: multi-leg serve — a destination that suspends
                    // mid-import (§11.2) closes the wire at a chunk boundary
                    // and resumes on a FRESH connection; the source re-serves.
                    val maxLegs = serveLegs.coerceAtLeast(1)
                    var legsUsed = 0
                    var sent = 0L
                    var recv = 0L
                    var importComplete = false
                    while (!importComplete && legsUsed < maxLegs) {
                        legsUsed++
                        Session.connect(addr, id, trust).use { wire ->
                            val theirs = wire.clientNegotiate(
                                JSONArray(descriptor.features()).toString()
                            )
                            if (legsUsed == 1) {
                                emitter(HandoffEvent.Console("negotiate:  descriptor sent (data only, §3.8); theirs: $theirs"))
                                emitter(HandoffEvent.State("Importing"))
                            }
                            try {
                                core.serveCut(wire)
                                val (s, r) = wire.bytes() // between completed ops only
                                sent += s
                                recv += r
                                emitter(HandoffEvent.Counters(sent, recv))
                                emitter(HandoffEvent.Console("serve:  cut served — destination reported ImportComplete (legs=$legsUsed)"))
                                emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.OK))
                                emitter(HandoffEvent.State("ActionReady"))
                                emitter(HandoffEvent.Console("L1:  launcher activation runs on the receiver (engine document.open + OS ACTION_VIEW)"))
                                importComplete = true
                            } catch (e: Exception) {
                                val (s, r) = wire.bytes()
                                sent += s
                                recv += r
                                emitter(HandoffEvent.Console("serve_leg:  leg $legsUsed closed mid-transfer ($e) — remaining legs ${maxLegs - legsUsed}"))
                                if (legsUsed >= maxLegs) throw e
                                emitter(HandoffEvent.Console("serve_leg:  re-connecting for the destination's resume (§11.2)"))
                            }
                        }
                    }
                    session
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
        /** Stage 6: whose identity this is — the durable Keystore identity is
         *  BORROWED (its owner closes it); a per-arm generated identity is
         *  closed with the arm (the 2026-11-08 double-free device-bug class). */
        val identityOwner: IdentityOwner = IdentityOwner.CALLER,
    ) : java.io.Closeable {
        /** Truncated pin for §22.3-style display. */
        val shortPin: String get() = pinHex.take(16) + "…"

        override fun close() {
            runCatching { listener.close() }
            // The identity is BORROWED when it is the durable store's (the
            // Keystore owner closes it) — a double free here would be the
            // 2026-11-08 device-bug class. Per-arm identities close inline.
            if (identityOwner == IdentityOwner.CALLER) {
                runCatching { identity.close() }
            }
            runCatching { core.close() }
        }
    }

    /** Who frees an Armed receiver's identity (stage-6 durable rule). */
    enum class IdentityOwner { CALLER, DURABLE_STORE }

    /**
     * Arm a receiver: open the core, bind the listener, display. Stage 6: the
     * identity is normally the DURABLE Keystore one (`topology.durableIdentity()`)
     * so the pin is stable across launches and pairing (stage 5) actually
     * matches; `identity == null` keeps the per-arm generated identity
     * (loopback tests, manual fallback). Cheap and cancellable — nothing
     * blocks here.
     */
    suspend fun arm(
        workDir: File,
        identity: Identity? = null,
    ): Armed = withContext(Dispatchers.IO) {
        workDir.mkdirs()
        val core = Core.open(workDir.resolve("dest").absolutePath)
        val id = identity ?: Identity.generate("carryon-receiver")
        val listener = Listener.bind("0.0.0.0:0")
        val pin = id.pinHex()
        val wireAddr = listener.addr()
        val port = wireAddr.substringAfterLast(':')
        Armed(
            core = core, identity = id, listener = listener, pinHex = pin,
            wireAddr = wireAddr,
            displayAddr = "${siteLocalIpv4() ?: "<no-wifi-ip>"}:$port",
            identityOwner = if (identity == null) IdentityOwner.CALLER else IdentityOwner.DURABLE_STORE,
        )
    }

    /**
     * The full stage-6 receive: accept (BLOCKING until the source connects),
     * negotiate (capture the descriptor), import — with the cooperative
     * suspend/resume legs (§11.2) — then the CORE-004 gate, reconstruction,
     * the engine's document.open record, the OS ACTION_VIEW, and the evidence
     * bundle. `launcher` performs the last step (platform side of L1).
     *
     * Trust (§18.5): `sourcePin` non-null = the MANUAL FALLBACK (a temp trust
     * pinning the typed pin); `sourcePin` null = the PAIRED durable store
     * (`durableTrust`) — no typing, and an unpaired/revoked source fails the
     * TLS handshake closed.
     *
     * Drills (honest, visible outcomes — never faked success):
     * `suspendDrill` requests a cooperative suspend before the first chunk
     * (the import returns Suspended with a resume token; the flow accepts a
     * resume leg and completes — §11.2); `tinyBudget` sets a §6.8 net budget
     * that makes the import refuse/partial — the run reports the refusal.
     *
     * `bytesBeforeFirstAction` (wire recv before the first action) and the
     * leg count ride into the result + evidence (§5.6 measured discipline).
     */
    suspend fun receive(
        armed: Armed,
        sourcePin: String?,
        chunkSize: Long,
        workDir: File,
        launcher: (bytes: ByteArray, fileName: String, mime: String) -> Pair<String, String?>,
        durableTrust: Trust? = null,
        suspendDrill: Boolean = false,
        tinyBudget: Boolean = false,
        emitter: (HandoffEvent) -> Unit = {},
    ): ReceiveResult = withContext(Dispatchers.IO) {
        val core = armed.core
        val emitterLog: (String, String) -> Unit = { k, v -> emitter(HandoffEvent.Console("$k:  $v")) }
        val paired = sourcePin == null

        emitter(HandoffEvent.Console("== Carry-On Android receive — L0/L1 (stage 6: discovery → pairing → import → action) =="))
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
        emitterLog("pin", "${armed.shortPin}  (durable device identity; public fingerprint)")

        // ---- trust: the PAIRED durable store, or the typed manual fallback ----
        val activeTrust: Trust = if (paired) {
            val t = durableTrust
                ?: throw CoreException(Err.E_AUTH_BASE + 0, "no paired trust store — pair first (stage 5) or type the source pin (manual fallback)")
            val records = t.records()
            if (records.isEmpty()) {
                throw CoreException(Err.E_AUTH_BASE + 0, "no paired peers in the durable store — pair first (stage 5) or use the manual pin")
            }
            emitterLog("trust", "PAIRED durable store (${records.size} record(s), Keystore-sealed) — unpaired/revoked sources fail the handshake closed (§18.5)")
            t
        } else {
            require(HandoffValidation.isValidPinHex(sourcePin)) {
                "source pin must be the 64-hex fingerprint shown on the source screen"
            }
            Trust.empty().also { it.pair("source", sourcePin, utcNow()) }
                .also { emitterLog("trust", "source pin ${sourcePin.take(16)}… paired (manual fallback, §18.4)") }
        }
        emitterLog("listener", "bound ${armed.wireAddr}  (display ${armed.displayAddr}; TLS 1.3, mutual pinning §18.5)")

        emitter(HandoffEvent.State("Importing"))
        emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.RUNNING))
        val tImport0 = System.nanoTime()

        // ---- §6.8 budget drill (visible refusal, never faked success) --------
        if (tinyBudget) {
            core.setBudget(
                JSONObject().apply {
                    put("total_net_bytes", 256)          // intentionally tiny
                    put("avg_net_rate", 1024)
                    put("burst_bytes", 256)
                    put("total_cpu_ms", 60_000)
                    put("cpu_duty", 1.0)
                    put("peak_prep_mem", 1_000_000)
                    put("storage_quota", 50_000_000)
                    put("battery_thermal", 1.0)
                    put("elapsed_opportunity_ms", 60_000)
                    put("max_nonpreemptible_ms", 250)
                }.toString()
            )
            emitterLog("budget", "tiny net budget set (§6.8 drill) — expect a VISIBLE refusal/partial, never a silent pass")
        }

        // ---- leg 1: accept the source (BLOCKING), negotiate, import ----------
        var wire = Session.accept(armed.listener, armed.identity, activeTrust)
        var legs = 1
        var sent = 0L
        var recv = 0L
        var currentSession: Session = wire
        val descriptor: HandoffDescriptor
        try {
            // The descriptor arrives in the source's Hello features (data
            // only, §3.8). Parse BEFORE anything else; refuse closed.
            descriptor = HandoffDescriptor.parse(currentSession.serverNegotiate("[]").let { jsonList(it) })
                ?: throw CoreException(Err.ERR_BAD_JSON, "handoff descriptor missing/malformed — refusing (§3.8)")
        } catch (t: Throwable) {
            currentSession.close()
            throw t
        }
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

        // ---- import — with the cooperative suspend/resume legs (§11.2) -------
        if (suspendDrill) {
            core.requestSuspend()
            emitterLog("suspend", "cooperative suspend requested — the import will checkpoint at the next chunk boundary and resume on a fresh connection (§11.2)")
        }
        var outcome = core.importCut(currentSession, descriptor.session, descriptor.cut)
        while (outcome is ImportOutcome.Suspended) {
            // Counters ONLY between completed wire ops (never during one).
            val (s1, r1) = currentSession.bytes()
            sent += s1
            recv += r1
            currentSession.close() // the suspended wire closes at the boundary
            legs++
            emitterLog("suspended", "leg ${legs - 1} checkpointed (§11.2) — accepting resume leg $legs…")
            emitter(HandoffEvent.State("Importing"))
            currentSession = Session.accept(armed.listener, armed.identity, activeTrust)
            jsonList(currentSession.serverNegotiate("[]")) // the source re-dials: Hello/Welcome again
            outcome = core.resumeImport(currentSession, outcome.resumeTokenJson)
        }
        wire = currentSession
        val (s2, r2) = wire.bytes() // between completed ops only
        sent += s2
        recv += r2
        emitter(HandoffEvent.Counters(sent, recv))
        val ttaMs = (System.nanoTime() - tImport0) / 1e6
        val mirrorCut = when (outcome) {
            is ImportOutcome.Completed -> {
                emitterLog("import", "completed (mirror_cut=${outcome.mirrorCut}; legs=$legs)")
                outcome.mirrorCut
            }
            is ImportOutcome.PartialBudget -> {
                emitterLog("budget", "§6.8 progressive admission: published=${outcome.published} deferred=${outcome.deferred} — a NORMAL outcome, nothing sealed")
                emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.FAILED))
                emitter(HandoffEvent.State("Failed"))
                throw CoreException(Err.E_BUDGET_BASE + 0, "import returned partial_budget (published=${outcome.published}, deferred=${outcome.deferred}) — the tiny-budget drill refused the full carry (§6.8, visible)")
            }
            else -> throw CoreException(Err.E_TRANSFER_BASE + 5, "import did not complete: $outcome")
        }
        emitterLog("bytes", "sent=$sent recv=$recv (MEASURED, §5.6; legs=$legs)")
        emitterLog("objects", "1/1 object carried (${descriptor.fileName})")
        emitterLog("time_to_action_ready", "%.1f ms".format(ttaMs))

        // ---- CORE-004 app-level gate: declared == measured, or REFUSE.
        // The engine guarantees the store (stage → verify → atomic publish);
        // this gate guards the app's reconstruction path and refuses closed.
        // NO success on a mismatched object, ever.
        val mirror = mirrorSessionId(descriptor.session)
        emitterLog("mirror_session", mirror)
        val present = core.hasObjectHex(descriptor.contentHash)
        if (!present) {
            emitterLog("gate", "declared object ABSENT in local store → REFUSED (fail closed, CORE-004)")
            emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.FAILED))
            emitter(HandoffEvent.State("Failed"))
            wire.close()
            throw CoreException(Err.E_OBJECT_BASE + 5, "declared content hash ${descriptor.contentHash.take(16)}… not present after import — REFUSED (CORE-004)")
        }
        emitterLog("L0", "object ${descriptor.contentHash.take(16)}… present in local store (source-independent)")
        val bytes = core.readObjectBytes(descriptor.contentHash)
        if (!IntegrityGate.verify(bytes, descriptor.contentHash)) {
            emitterLog("gate", "sha256 measured != declared → REFUSED (fail closed, CORE-004)")
            emitter(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.FAILED))
            emitter(HandoffEvent.State("Failed"))
            wire.close()
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
        // bytes_before_first_action: the wire traffic before the FIRST action
        // completed (the sample stays between completed ops — the open adds
        // no wire traffic; wire recv is the honest pre-action measure).
        val bytesBeforeFirstAction = recv
        wire.close()

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
            put("bytes_before_first_action", bytesBeforeFirstAction)
            put("serve_legs_used", legs)
            put("paired_trust", paired)
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
            bytesBeforeFirstAction = bytesBeforeFirstAction,
            legsUsed = legs,
            pairedTrust = paired,
        )
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
