/*
 * DemoRunner.kt — Demo Mode: the REAL engine through the C ABI, TWO independent
 * cores in temp dirs on ONE device, real TLS 1.3 loopback, mutual pin pairing.
 *
 * Claim label (§2 claim law): **OBSERVED (LOCAL)** — one device, loopback TLS 1.3.
 * This is NOT physical cross-device evidence (spec §2/§30) and no platform is
 * "supported". The disclosure footer is carried verbatim in DemoState.
 *
 * 8-stage pipeline (plan.md Demo Mode), each stage an ASCII block in the UI:
 *
 *   1 OPEN      carryon_core_open x2 (temp data dirs) [+ toggles: chunk size,
 *               tiny budget, recovery report]
 *   2 IDENTITY  carryon_identity_generate x2 — truncated pin fingerprints only
 *   3 PAIR      carryon_pair_devices -> trust_from_json each side
 *   4 SESSION   register_adapter (compiled-in adapter by id+JSON, §3.8),
 *               create_session, create_cut
 *   5 TRANSFER  listener_bind("127.0.0.1:0") -> accept/connect -> negotiate ->
 *               serve_cut <=> import_cut with live session_bytes counters
 *   6 AUTHORITY may_mutate(false on mirror) -> request/serve_authority_transfer
 *               (SAME wire as the completed import, the carryon_l3.c flow) ->
 *               receipt (new_epoch) -> may_mutate flips: dest true, src false
 *   7 EDIT      dest execute_action_as source-off (single-writer L4, §21.2);
 *               oracle `agreed` shown
 *   8 EVIDENCE  export_evidence_with (l3-style measured metrics) ->
 *               verify_evidence -> {"ok":true,...}
 *
 * Wire-round protocol (deterministic, mirrors crates/carryon-ffi tests):
 *   - The source runs a bounded serve loop: per round accept -> negotiate ->
 *     serve_cut; on the FINAL round the same session also carries
 *     serve_authority_transfer (l3 flow). A suspended/partial round closes the
 *     wire and the dest reconnects fresh (round_trip.rs suspend/resume flow).
 *   - Suspend toggle: request_suspend BEFORE the first chunk so round 1 always
 *     suspends; resume_import on a fresh connection completes.
 *   - Budget toggle: tiny net budget -> typed E_BUDGET refusal (or the ABI 1.1
 *     `partial_budget` shape) — a NORMAL visible outcome (§6.8); replenish and
 *     retry on a fresh connection (dedup skips published objects).
 *
 * Threading + lifetime: every blocking engine call (serve_cut, import_cut,
 * accept, the authority RPCs) runs on Dispatchers.IO. JNI calls are NOT
 * interruptible: if the pipeline fails, the handles in use by blocked calls are
 * deliberately LEAKED (freeing under a blocked call is use-after-free); they die
 * with the process and each run uses fresh temp dirs. The whole pipeline is
 * bounded by DEMO_TIMEOUT_MS so the UI cannot hang forever.
 */
package com.carryon.app.demo

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
import com.carryon.app.ffi.mayMutate
import com.carryon.app.ffi.mirrorSessionId
import com.carryon.app.ffi.pairDevices
import com.carryon.app.ffi.recoveryReport
import com.carryon.app.ffi.requestAuthorityTransfer
import com.carryon.app.ffi.resumeImport
import com.carryon.app.ffi.serveAuthorityTransfer
import com.carryon.app.ffi.serveCut
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.atomic.AtomicLong

/* ---- public model (UI consumes this) -------------------------------------- */

/** The two scenarios (plan.md Demo Mode). Adapter ids + params are DATA naming
 *  compiled-in adapters (§3.8) — never behavior. */
enum class Scenario(
    val label: String,
    val adapterId: String,
    val adapterParams: String,
    val sessionTitle: String,
    val authoritativeObjects: List<String>,
    val optionalObjects: List<String>,
) {
    EDITOR_L3(
        label = "L3 editor handoff",
        adapterId = "org.carryon.editor",
        adapterParams = "{\"session_v1\":true}",
        sessionTitle = "carryon-demo-editor",
        authoritativeObjects = listOf(
            "editor.document.v1", "editor.unsaved_edits.v1", "editor.meta.v1"
        ),
        optionalObjects = listOf("editor.navigation.v1"),
    ),
    GRAPH(
        label = "Graph handoff (L2 typed objects)",
        adapterId = "org.carryon.graph",
        adapterParams = "{\"sample\":true}",
        sessionTitle = "carryon-demo-graph",
        authoritativeObjects = listOf("graph.nodes.v1", "graph.edges.v1"),
        optionalObjects = listOf("graph.adjacency_index.v1"),
    ),
}

/** The 8 pipeline stages. */
enum class Stage(val label: String) {
    OPEN("OPEN"),
    IDENTITY("IDENTITY"),
    PAIR("PAIR"),
    SESSION("SESSION"),
    TRANSFER("TRANSFER"),
    AUTHORITY("AUTHORITY"),
    EDIT("EDIT"),
    EVIDENCE("EVIDENCE"),
}

enum class StageStatus { PENDING, RUNNING, DONE, FAILED }

data class StageUi(
    val stage: Stage,
    val status: StageStatus = StageStatus.PENDING,
    val detail: String = "",
)

/** Advanced showcase toggles (plan.md Demo settings). */
data class DemoToggles(
    /** 0 = engine default; else chunk bytes (e.g. 256) — progressive transfer. */
    val chunkSize: Long = 0,
    /** Tiny net budget -> typed E_BUDGET refusal or `partial_budget` — NORMAL
     *  outcome (§6.8: budget rejection SHALL be visible), then replenish+retry. */
    val budgetRejection: Boolean = false,
    /** request_suspend before import -> `{"suspended":true,resume_token}` ->
     *  resume_import on a FRESH connection completes (§11.2). */
    val suspendResume: Boolean = false,
    /** Log `carryon_core_recovery_report_json` fields (§19.3). */
    val recoveryReport: Boolean = false,
)

data class DemoState(
    val running: Boolean = false,
    val scenario: Scenario = Scenario.EDITOR_L3,
    val toggles: DemoToggles = DemoToggles(),
    val stages: List<StageUi> = Stage.entries.map { StageUi(it) },
    /** Monospace engine console lines, repo style `key:  value`. */
    val console: List<String> = emptyList(),
    val bytesSent: Long = 0,
    val bytesRecv: Long = 0,
    val sourcePin: String = "",
    val destPin: String = "",
    /** FULL pin hex (public material — EVD-006 excludes only private keys). Used
     *  to record the real pairing in the app trust store; the UI truncates. */
    val sourcePinHex: String = "",
    val destPinHex: String = "",
    val listenerAddr: String = "",
    val sourceMayMutate: Boolean? = null,
    val destMayMutate: Boolean? = null,
    val authorityEpoch: Long? = null,
    val evidenceBundleJson: String? = null,
    val verifyReportJson: String? = null,
    val error: String? = null,
    /** Elapsed wall-clock of the last completed run, ms. */
    val runElapsedMs: Long = 0,
) {
    companion object {
        /** Verbatim disclosure footer (plan.md — copied, never reworded). */
        const val DISCLOSURE =
            "DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and\n" +
                "engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30)."

        /** The run's claim label shown in the UI (plan.md Demo Mode). */
        const val RUN_LABEL =
            "OBSERVED (LOCAL) — one device, loopback TLS 1.3; not physical cross-device evidence (spec §2/§30)"
    }
}

/* ---- runner ------------------------------------------------------------------ */

class DemoRunner(
    private val scope: CoroutineScope,
    private val baseDir: File,
) {
    private val mutex = Mutex()
    private val _state = MutableStateFlow(DemoState())
    val state: StateFlow<DemoState> = _state.asStateFlow()

    fun setScenario(scenario: Scenario) =
        _state.update { if (it.running) it else it.copy(scenario = scenario) }

    fun setToggles(toggles: DemoToggles) =
        _state.update { if (it.running) it else it.copy(toggles = toggles) }

    /** Launch one full demo run (guarded: one at a time). */
    fun runDemo() {
        scope.launch {
            mutex.withLock {
                if (_state.value.running) return@withLock
                val started = System.nanoTime()
                _state.update {
                    it.copy(
                        running = true,
                        error = null,
                        stages = Stage.entries.map { s -> StageUi(s) },
                        console = emptyList(),
                        bytesSent = 0,
                        bytesRecv = 0,
                        sourcePin = "",
                        destPin = "",
                        sourcePinHex = "",
                        destPinHex = "",
                        listenerAddr = "",
                        sourceMayMutate = null,
                        destMayMutate = null,
                        authorityEpoch = null,
                        evidenceBundleJson = null,
                        verifyReportJson = null,
                    )
                }
                try {
                    withTimeout(DEMO_TIMEOUT_MS) { runPipeline() }
                    _state.update {
                        it.copy(runElapsedMs = (System.nanoTime() - started) / 1_000_000)
                    }
                } catch (e: CancellationException) {
                    if (e is kotlinx.coroutines.TimeoutCancellationException) {
                        failCurrentStage("demo timed out after ${DEMO_TIMEOUT_MS / 1000}s (blocked engine call)")
                    } else {
                        failCurrentStage("cancelled")
                        throw e
                    }
                } catch (e: CoreException) {
                    failCurrentStage("engine error ${Err.name(e.code)}: ${e.message}")
                } catch (e: Exception) {
                    failCurrentStage("unexpected error: ${e.message}")
                } finally {
                    _state.update { it.copy(running = false) }
                }
            }
        }
    }

    /* ---- console + stage helpers ---- */

    private fun log(key: String, value: String) {
        val line = "$key:  $value"
        _state.update { it.copy(console = (it.console + line).takeLast(MAX_CONSOLE_LINES)) }
    }

    private fun beginStage(stage: Stage) = _state.update { st ->
        st.copy(stages = st.stages.map {
            if (it.stage == stage) it.copy(status = StageStatus.RUNNING) else it
        })
    }

    private fun endStage(stage: Stage, detail: String) = _state.update { st ->
        st.copy(stages = st.stages.map {
            if (it.stage == stage) it.copy(status = StageStatus.DONE, detail = detail) else it
        })
    }

    private fun failCurrentStage(detail: String) {
        _state.update { st ->
            val failing = st.stages.firstOrNull { it.status == StageStatus.RUNNING }
            st.copy(
                stages = st.stages.map {
                    if (failing != null && it.stage == failing.stage) {
                        it.copy(status = StageStatus.FAILED, detail = detail)
                    } else {
                        it
                    }
                },
                error = detail,
                console = (st.console + "FAIL:  $detail").takeLast(MAX_CONSOLE_LINES),
            )
        }
    }

    /* ---- the pipeline ---- */

    private suspend fun runPipeline() {
        val s0 = _state.value
        val scenario = s0.scenario
        val toggles = s0.toggles

        val stamp = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date())
        val base = File(baseDir, "demo/$stamp").apply { mkdirs() }
        val srcDir = File(base, "src")
        val dstDir = File(base, "dst")

        var srcCore: Core? = null
        var dstCore: Core? = null
        var srcId: Identity? = null
        var dstId: Identity? = null
        var srcTrust: Trust? = null
        var dstTrust: Trust? = null
        var listener: Listener? = null

        // Accumulated MEASURED wire counters across (possibly several) connections.
        val totalSent = AtomicLong(0)
        val totalRecv = AtomicLong(0)

        try {
            // coroutineScope: the serve loop + poll jobs are STRUCTURED children —
            // a pipeline failure/timeout cancels them (their blocked JNI threads
            // drain when the peer disconnects; handles stay leaked per the rules).
            coroutineScope {
            /* ---- 1 OPEN ---- */
            beginStage(Stage.OPEN)
            val coreSrc = withContext(Dispatchers.IO) { Core.open(srcDir.absolutePath) }
            srcCore = coreSrc
            val coreDst = withContext(Dispatchers.IO) { Core.open(dstDir.absolutePath) }
            dstCore = coreDst
            val abi = abiVersion()
            log("abi", "engine ${abi.major}.${abi.minor} (expect 1.1)")
            log("core.src", "opened ${srcDir.absolutePath}")
            log("core.dst", "opened ${dstDir.absolutePath}")
            coreSrc.setForeground(true)
            coreDst.setForeground(true)
            log("foreground", "true on both cores (§20.2 admission)")
            if (toggles.chunkSize > 0) {
                coreSrc.setChunkSize(toggles.chunkSize)
                coreDst.setChunkSize(toggles.chunkSize)
                log("chunk_size", "${toggles.chunkSize} B (progressive transfer visible)")
            }
            if (toggles.budgetRejection) {
                coreDst.setBudget(TINY_BUDGET_JSON)
                log("budget.dst", "TINY net budget set (§6.8 rejection expected, then replenished)")
            }
            if (toggles.recoveryReport) {
                listOf("src" to coreSrc, "dst" to coreDst).forEach { (name, core) ->
                    val r = withContext(Dispatchers.IO) { core.recoveryReport() }
                    log(
                        "recovery.$name",
                        "interrupted=${r.interruptedTransfers} hidden=${r.hiddenIncompleteObjects}" +
                            " ambiguous=${r.ambiguousSessions} trunc=${r.journalTruncated} clean=${r.clean}"
                    )
                }
            }
            endStage(Stage.OPEN, "2 cores open (temp dirs)")

            /* ---- 2 IDENTITY ---- */
            beginStage(Stage.IDENTITY)
            val idSrc = withContext(Dispatchers.IO) { Identity.generate("carryon-src") }
            srcId = idSrc
            val idDst = withContext(Dispatchers.IO) { Identity.generate("carryon-dst") }
            dstId = idDst
            val srcPin = withContext(Dispatchers.IO) { idSrc.pinHex() }
            val dstPin = withContext(Dispatchers.IO) { idDst.pinHex() }
            log("pin.src", shortPin(srcPin))
            log("pin.dst", shortPin(dstPin))
            log("key.der", "SECRET — never logged/displayed (§22.3, EVD-006)")
            _state.update {
                it.copy(
                    sourcePin = shortPin(srcPin),
                    destPin = shortPin(dstPin),
                    sourcePinHex = srcPin,
                    destPinHex = dstPin,
                )
            }
            endStage(Stage.IDENTITY, "2 identities, pins pinned")

            /* ---- 3 PAIR ---- */
            beginStage(Stage.PAIR)
            val utc = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US).format(Date())
            val pairing = withContext(Dispatchers.IO) { pairDevices(idSrc, idDst, utc) }
            val trustSrc = withContext(Dispatchers.IO) { Trust.fromJson(pairing.localTrustJson) }
            srcTrust = trustSrc
            val trustDst = withContext(Dispatchers.IO) { Trust.fromJson(pairing.remoteTrustJson) }
            dstTrust = trustDst
            log("pairing", "mutual pin at $utc (trust_generation=0)")
            endStage(Stage.PAIR, "mutual pin pairing")

            /* ---- 4 SESSION ---- */
            beginStage(Stage.SESSION)
            val infoSrc = withContext(Dispatchers.IO) {
                coreSrc.registerAdapter(scenario.adapterId, scenario.adapterParams)
            }
            withContext(Dispatchers.IO) {
                coreDst.registerAdapter(scenario.adapterId, scenario.adapterParams)
            }
            log("adapter", "${scenario.adapterId} registered on both cores")
            log("adapter.info", infoSrc.take(120))
            val createReq = JSONObject().apply {
                put("adapter_id", scenario.adapterId)
                put("title", scenario.sessionTitle)
                put("privacy", "personal")
                put("authority_mode", "single_writer")
            }
            val sessionId = withContext(Dispatchers.IO) { coreSrc.createSession(createReq.toString()) }
            val cut = withContext(Dispatchers.IO) { coreSrc.createCut(sessionId) }
            log("session.src", sessionId)
            log("cut", "sealed number=${cut.number} (two-phase publish, CORE-004)")
            endStage(Stage.SESSION, "session + cut ${cut.number}")

            /* ---- 5 TRANSFER ---- */
            beginStage(Stage.TRANSFER)
            val lis = withContext(Dispatchers.IO) { Listener.bind("127.0.0.1:0") }
            listener = lis
            val addr = withContext(Dispatchers.IO) { lis.addr() }
            log("listener", "bound $addr (TLS 1.3 loopback)")
            _state.update { it.copy(listenerAddr = addr) }

            // Wire rounds are deterministic given the toggles (see file header):
            // suspend -> +1, budget rejection -> +1.
            val expectedRounds =
                1 + (if (toggles.suspendResume) 1 else 0) + (if (toggles.budgetRejection) 1 else 0)
            var serveLoopError: Throwable? = null

            // SOURCE: bounded serve loop. Final round: serve_cut, then the same
            // session carries serve_authority_transfer (l3 flow; the dest's
            // request below pairs with it).
            val serveLoop = launch(Dispatchers.IO) {
                try {
                    for (round in 1..expectedRounds) {
                        val sess = Session.accept(lis, idSrc, trustSrc)
                        sess.serverNegotiate()
                        coreSrc.serveCut(sess)
                        log("serve.round$round", "cut served")
                        if (round == expectedRounds) {
                            coreSrc.serveAuthorityTransfer(sess, sessionId, cut.number)
                            log("serve.authority", "relinquished; source read-only (§21.2)")
                        }
                        val (bs, br) = sess.bytes()
                        totalSent.addAndGet(bs)
                        totalRecv.addAndGet(br)
                        sess.close()
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Throwable) {
                    serveLoopError = e
                    log("serve", "serve loop failed: ${e.message}")
                }
            }

            // DESTINATION: connect -> negotiate -> import, honoring the toggles.
            val tImport0 = System.nanoTime()
            var wire: Session? = null
            var pollJob: Job? = null
            var outcome: ImportOutcome? = null
            var resumeToken: String? = null
            var suspendRequested = false
            var budgetReplenished = false
            var attempts = 0

            while (outcome !is ImportOutcome.Completed) {
                attempts++
                if (attempts > MAX_WIRE_ATTEMPTS) {
                    throw CoreException(Err.E_TRANSFER_BASE + 5, "wire attempts exceeded $MAX_WIRE_ATTEMPTS")
                }
                val sess = withContext(Dispatchers.IO) { Session.connect(addr, idDst, trustDst) }
                wire = sess
                withContext(Dispatchers.IO) { sess.clientNegotiate() }

                // Live MEASURED counters for the UI while this connection moves data.
                pollJob?.cancel()
                pollJob = launch {
                    while (isActive) {
                        try {
                            val (bs, br) = sess.bytes()
                            _state.update {
                                it.copy(
                                    bytesSent = totalSent.get() + bs,
                                    bytesRecv = totalRecv.get() + br,
                                )
                            }
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            // session closed mid-poll — counters stop
                        }
                        delay(BYTE_POLL_MS)
                    }
                }

                if (toggles.suspendResume && !suspendRequested) {
                    // BEFORE the first chunk so round 1 always suspends
                    // (deterministic; see round_trip.rs).
                    withContext(Dispatchers.IO) { coreDst.requestSuspend() }
                    suspendRequested = true
                    log("suspend", "requested before first chunk (§11.2)")
                }

                outcome = if (resumeToken != null) {
                    val token = resumeToken
                    resumeToken = null
                    withContext(Dispatchers.IO) { coreDst.resumeImport(sess, token!!) }
                } else {
                    try {
                        withContext(Dispatchers.IO) {
                            coreDst.importCut(sess, sessionId, cut.number)
                        }
                    } catch (e: CoreException) {
                        if (toggles.budgetRejection && e.code == Err.E_BUDGET_BASE) {
                            null // typed refusal before bytes moved — handled below
                        } else {
                            throw e
                        }
                    }
                }

                when (val o = outcome) {
                    is ImportOutcome.Completed -> {
                        log("import", "completed (mirror_cut=${o.mirrorCut})")
                    }
                    is ImportOutcome.Suspended -> {
                        log(
                            "import",
                            "SUSPENDED mid-object, resume_token issued (interrupted ≠ success, §19.3.9)"
                        )
                        resumeToken = o.resumeTokenJson
                        outcome = null
                    }
                    is ImportOutcome.PartialBudget -> {
                        log(
                            "import",
                            "PARTIAL-BUDGET prefix: published=${o.published}" +
                                " deferred=${o.deferred} (${o.deferredBytes} B) — §6.8"
                        )
                        outcome = null
                    }
                    null -> {
                        log("budget", "import REFUSED (E_BUDGET_NETWORK) — normal outcome, §6.8")
                    }
                }

                if (outcome == null) {
                    // This round's wire is done: tally, cancel the poll, close.
                    // (Suspended/partial/refused wires are closed by protocol;
                    // the resume/retry reconnects fresh.)
                    pollJob?.cancel()
                    pollJob = null
                    val (bs, br) = sess.bytes()
                    totalSent.addAndGet(bs)
                    totalRecv.addAndGet(br)
                    _state.update {
                        it.copy(bytesSent = totalSent.get(), bytesRecv = totalRecv.get())
                    }
                    sess.close()
                    if (wire === sess) wire = null
                    if (toggles.budgetRejection && !budgetReplenished) {
                        withContext(Dispatchers.IO) { coreDst.setBudget(GENEROUS_BUDGET_JSON) }
                        budgetReplenished = true
                        log("budget", "replenished; retrying (published objects skipped by dedup)")
                    }
                }
            }
            val tActionReady = System.nanoTime()
            val ttaMs = (tActionReady - tImport0) / 1e6
            endStage(Stage.TRANSFER, "mirror cut ${cut.number} imported")

            /* ---- 6 AUTHORITY (same wire as the completed import — l3 flow) ---- */
            beginStage(Stage.AUTHORITY)
            val mirror = withContext(Dispatchers.IO) { mirrorSessionId(sessionId) }
            log("mirror_session", mirror)
            val cutRef = JSONObject().apply {
                put("session", mirror)
                put("number", cut.number)
            }
            // Read-only oracle first: the SAME logical session restored (l3 flow).
            var restoreAgreed = false
            if (scenario == Scenario.EDITOR_L3) {
                val tRestore0 = System.nanoTime()
                val restore = withContext(Dispatchers.IO) {
                    coreDst.executeActionAs(
                        cutRef.toString(),
                        "{\"class\":\"session.restore\",\"params\":{}}",
                        scenario.adapterId,
                    )
                }
                restoreAgreed = JSONObject(restore)
                    .optJSONObject("oracle")?.optBoolean("agreed") ?: false
                log(
                    "session.restore",
                    "oracle agreed=$restoreAgreed " +
                        "(${String.format(Locale.US, "%.1f ms", (System.nanoTime() - tRestore0) / 1e6)})"
                )
            }
            val preMutate = withContext(Dispatchers.IO) { coreDst.mayMutate(mirror) }
            log("may_mutate.dst.pre", preMutate.toString())
            if (preMutate) {
                throw CoreException(Err.E_AUTH_BASE + 2, "mirror must not start authoritative")
            }

            val authSess = wire ?: throw CoreException(Err.E_BAD_HANDLE, "import wire closed")
            val tAuth0 = System.nanoTime()
            // The source is inside serve_authority_transfer (final serve round);
            // this request drives it. Durable receipts BEFORE the epoch moves
            // (AUTH-004).
            val receipt = withContext(Dispatchers.IO) {
                coreDst.requestAuthorityTransfer(authSess, mirror, scenario.adapterId)
            }
            val epoch = JSONObject(receipt).optLong("new_epoch", -1)
            val srcIndependentMs = (System.nanoTime() - tAuth0) / 1e6
            log("authority", "receipt epoch=$epoch")
            _state.update { it.copy(authorityEpoch = epoch) }

            // Close the wire; the source's serve_authority_transfer returns,
            // tallies its counters and exits the serve loop.
            pollJob?.cancel()
            pollJob = null
            val (finalSent, finalRecv) = authSess.bytes()
            totalSent.addAndGet(finalSent)
            totalRecv.addAndGet(finalRecv)
            authSess.close()
            wire = null
            _state.update {
                it.copy(bytesSent = totalSent.get(), bytesRecv = totalRecv.get())
            }
            serveLoop.join()
            serveLoopError?.let { throw it }
            log("bytes", "sent=${totalSent.get()} recv=${totalRecv.get()} (MEASURED, §5.6)")
            log("time_to_action_ready", String.format(Locale.US, "%.1f ms", ttaMs))

            val postDst = withContext(Dispatchers.IO) { coreDst.mayMutate(mirror) }
            val postSrc = withContext(Dispatchers.IO) { coreSrc.mayMutate(sessionId) }
            log("may_mutate.dst.post", postDst.toString())
            log("may_mutate.src.post", postSrc.toString())
            if (!postDst || postSrc) {
                throw CoreException(Err.E_AUTH_BASE, "single-writer flip violated (§21.2)")
            }
            _state.update {
                it.copy(destMayMutate = postDst, sourceMayMutate = postSrc)
            }
            endStage(Stage.AUTHORITY, "epoch=$epoch · dest owner · src read-only")

            /* ---- 7 EDIT (source-off independent continuation) ---- */
            beginStage(Stage.EDIT)
            // Both transports are closed: the dest continues ALONE (L4 proof).
            val editReq = when (scenario) {
                Scenario.EDITOR_L3 -> JSONObject().apply {
                    put("class", "document.edit")
                    put(
                        "params",
                        JSONObject().put(
                            "text",
                            "continued on the Android device, source gone [carry-on-app]"
                        ),
                    )
                }
                Scenario.GRAPH -> JSONObject().apply {
                    put("class", "graph.shortest_path")
                    put("params", JSONObject().put("start", 0).put("end", 5))
                }
            }
            val tEdit0 = System.nanoTime()
            val edit = withContext(Dispatchers.IO) {
                coreDst.executeActionAs(cutRef.toString(), editReq.toString(), scenario.adapterId)
            }
            val editMs = (System.nanoTime() - tEdit0) / 1e6
            val editAgreed = JSONObject(edit)
                .optJSONObject("oracle")?.optBoolean("agreed") ?: false
            log(
                "edit",
                "${editReq.getString("class")} oracle agreed=$editAgreed " +
                    "(${String.format(Locale.US, "%.1f ms", editMs)})"
            )
            if (!editAgreed) {
                throw CoreException(Err.E_ACTION_BASE + 3, "independent-edit oracle disagreed")
            }
            endStage(Stage.EDIT, "source-off edit agreed")

            /* ---- 8 EVIDENCE ---- */
            beginStage(Stage.EVIDENCE)
            val extra = JSONObject().apply {
                put(
                    "level",
                    if (scenario == Scenario.EDITOR_L3) "L3-structured-continuation"
                    else "L2-typed-objects"
                )
                put("mirror_session_id", mirror)
                put("mirror_cut", cut.number)
                put("source_session_id", sessionId)
                put("bytes_transferred_sent", totalSent.get())
                put("bytes_transferred_recv", totalRecv.get())
                put("bytes_recv_to_action_ready", totalRecv.get())
                put("time_to_action_ready_ms", ttaMs)
                put("restore_latency_ms", if (scenario == Scenario.EDITOR_L3) ttaMs else 0.0)
                put("source_independence_ms", srcIndependentMs)
                put("independent_edit_ms", editMs)
                put("authoritative_objects", JSONArray(scenario.authoritativeObjects))
                put("optional_objects", JSONArray(scenario.optionalObjects))
                put("oracle_restore_agreed", restoreAgreed)
                put("oracle_independent_edit_agreed", editAgreed)
                put(
                    "endpoint_changed_symbols_note",
                    "separate symbol-distance metric (spec §5.6); not bytes/runtime"
                )
            }
            val bundle = withContext(Dispatchers.IO) {
                coreDst.exportEvidenceWith(mirror, extra.toString())
            }
            val bundleFile = File(dstDir, "evidence.json").apply { writeText(bundle) }
            log("evidence", "exported ${bundle.length} B -> ${bundleFile.name}")
            val report = withContext(Dispatchers.IO) {
                coreDst.verifyEvidence(bundleFile.absolutePath)
            }
            val ok = JSONObject(report).optBoolean("ok", false)
            log("verify_evidence", report.take(160))
            if (!ok) {
                throw CoreException(Err.E_INTERNAL_BASE, "evidence verification did not report ok")
            }
            endStage(Stage.EVIDENCE, "verified ok=true")
            log("result", "pipeline complete — OBSERVED (LOCAL), spec §2/§30")

            _state.update {
                it.copy(evidenceBundleJson = bundle, verifyReportJson = report)
            }

            // Success: everything is quiescent — close deterministically.
            runCatching { srcTrust?.close() }
            runCatching { dstTrust?.close() }
            runCatching { srcId?.close() }
            runCatching { dstId?.close() }
            runCatching { listener?.close() }
            runCatching { srcCore?.close() }
            runCatching { dstCore?.close() }
            }
        } finally {
            // Failure path: handles held by blocked JNI calls are deliberately
            // leaked (freeing under a blocked call is use-after-free). Temp dirs
            // live in cacheDir; the process owns them.
        }
    }

    private fun shortPin(pinHex: String): String = pinHex.take(16) + "…"

    companion object {
        private const val MAX_CONSOLE_LINES = 400
        private const val BYTE_POLL_MS = 150L
        private const val DEMO_TIMEOUT_MS = 120_000L
        private const val MAX_WIRE_ATTEMPTS = 5

        /** §6.8 budget JSON (model/budget.rs serde shape) — tiny net allowance so
         *  the import is refused/partial: a NORMAL, visible outcome. */
        private const val TINY_BUDGET_JSON =
            "{\"total_net_bytes\":256,\"avg_net_rate\":256,\"burst_bytes\":256," +
                "\"total_cpu_ms\":60000,\"cpu_duty\":1.0,\"peak_prep_mem\":536870912," +
                "\"storage_quota\":1099511627776,\"battery_thermal\":\"Unrestricted\"," +
                "\"elapsed_opportunity_ms\":60000,\"max_nonpreemptible_ms\":5000}"

        /** Generous budget — the replenished/retry baseline. */
        private const val GENEROUS_BUDGET_JSON =
            "{\"total_net_bytes\":1073741824,\"avg_net_rate\":104857600,\"burst_bytes\":10485760," +
                "\"total_cpu_ms\":60000,\"cpu_duty\":1.0,\"peak_prep_mem\":536870912," +
                "\"storage_quota\":1099511627776,\"battery_thermal\":\"Unrestricted\"," +
                "\"elapsed_opportunity_ms\":60000,\"max_nonpreemptible_ms\":5000}"
    }
}
