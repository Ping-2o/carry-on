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
 *               serve_cut <=> import_cut; session_bytes sampled only BETWEEN
 *               completed wire ops (a CarryonSession is NOT thread-safe —
 *               never read while a blocking JNI call is in flight on it)
 *   6 AUTHORITY may_mutate(false on mirror) -> request/serve_authority_transfer
 *               (SAME wire as the completed import, the carryon_l3.c flow;
 *               EDITOR_L3 only — org.carryon.graph declares
 *               supports_authority_transfer:false, so GRAPH skips honestly,
 *               mirroring DemoPipeline.swift) ->
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
 *   - EXACT ROUND ORDER (device-run bug 2026-11-07: with suspend+budget BOTH
 *     on, the source expected 3 rounds but the dest completed in 2 — the armed
 *     suspend preempts the budget refusal (§11.2 fires before §6.8 moves any
 *     bytes), the replenish at the suspend round's close then removed the
 *     budget round entirely, while the source's serve loop ALSO died at the
 *     first mid-serve wire close (E_PROTO_FRAMING "connection closed" —
 *     round_trip.rs IGNORES the serve_cut result for exactly this reason)
 *     and round 2 hung in accept until the watchdog). Deterministic order,
 *     one wire per showcase:
 *       round 1: budget refusal ONLY (§6.8; suspend deliberately NOT armed
 *                yet — it would preempt the refusal); replenish at close;
 *       round 2: suspend (§11.2, armed before its import's first chunk),
 *                resume_token issued, wire closed; resume reconnects fresh;
 *       round 3 (final): resume_import completes; EDITOR_L3 authority moves
 *                on this same wire. GRAPH: cut serve alone (honest skip).
 *     Each toggle contributes exactly one extra wire, so the source's
 *     expectedRounds = 1 + suspend + budget matches the dest's actual flow.
 *   - The serve loop TOLERATES a mid-serve wire close on NON-final rounds:
 *     that close IS the §11.2 suspend / §6.8 refusal signal (the reference
 *     server ignores serve_cut errors and just accepts the next connection).
 *     A mid-serve close on the FINAL round, or an accept/negotiate failure,
 *     remains fatal.
 *
 * Threading + lifetime: every blocking engine call (serve_cut, import_cut,
 * accept, the authority RPCs) runs on Dispatchers.IO. JNI calls are NOT
 * interruptible: if the pipeline fails, the handles in use by blocked calls are
 * deliberately LEAKED (freeing under a blocked call is use-after-free); they die
 * with the process and each run uses fresh temp dirs.
 *
 * Timeout honesty (review finding): the timeout watchdog bounds the UI STATE
 * MACHINE, not the native calls. A blocked JNI call cannot be interrupted by
 * coroutine cancellation, the C ABI has no cancellation-safe abort entry
 * (freeing a handle under a blocked call is use-after-free), and structured
 * concurrency means the OLD withTimeout{runPipeline()} could hang inside its
 * own catch/finally — coroutineScope waits for the serveLoop child parked in
 * accept(), so the timeout might never even transition the run to FAILED.
 * The watchdog (runDemo) instead: cancels the pipeline BEST-EFFORT on
 * timeout, marks the run FAILED immediately, and releases the UI (mutex +
 * running flag). The wedged coroutine + its handles LEAK and drain with the
 * process; a fresh run uses fresh temp dirs, so a leaked wedge does not
 * poison it. Restarting the app remains the only way to clear the wedge
 * itself. Do not promise more than that anywhere in the UI copy.
 *
 * Zombie isolation (review finding): when the old pipeline's blocked JNI call
 * finally returns — after the timeout, possibly after a NEWER run has
 * started — it must not touch the newer run's state. Every pipeline state
 * write goes through a per-run RunUI handle generation-checked against
 * runGeneration; stale writes are dropped, and the cancelled pipeline's
 * CancellationException is rethrown, never reported as a failure.
 *
 * Counter discipline (review finding: the old pollJob sampled session_bytes
 * concurrently with import_cut and cancel() was not joined before close()):
 * a CarryonSession handle is NOT thread-safe (carryon.h), so counters are read
 * ONLY between completed wire operations — never during, never from a poller.
 */
package com.carryon.app.demo

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
import dev.carryon.kit.requestAuthorityTransfer
import dev.carryon.kit.resumeImport
import dev.carryon.kit.serveAuthorityTransfer
import dev.carryon.kit.serveCut
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
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
                // Timeout watchdog (review finding: the OLD
                // withTimeout { runPipeline() } could hang INSIDE its own
                // catch/finally — runPipeline's coroutineScope waits for the
                // serveLoop child parked in a blocking JNI accept(), the
                // cancellation never surfaces, and the run never even reached
                // FAILED). The watchdog therefore bounds the UI STATE MACHINE,
                // not the native calls:
                //  - runPipeline runs as a child job; on timeout it is
                //    cancelled BEST-EFFORT (a blocked JNI call is interrupted
                //    only at its next suspension point — often never).
                //  - the run is marked FAILED IMMEDIATELY and the mutex +
                //    `running` flag are released, so the app stays usable.
                //  - the wedged coroutine + its handles LEAK (freeing under a
                //    blocked call is use-after-free) and drain with the
                //    process; a leaked wedge cannot poison the next run
                //    (fresh temp dirs + fresh listener). Restarting the app
                //    remains the only way to clear the leaked wedge itself.
                // Generation token (review finding): after a watchdog timeout
                // the mutex is released and the user may start a NEW run while
                // this pipeline's blocked JNI call is still parked in the
                // engine. Every state write from pipeline code carries this
                // run's generation and is DROPPED once a newer run exists —
                // a zombie must never corrupt the newer run's state.
                val ui = RunUI(nextGeneration())
                val pipeline = launch {
                    try {
                        runPipeline(ui)
                    } catch (e: CancellationException) {
                        // CancellationException inherits from Exception, so
                        // the generic catch below would otherwise run
                        // failCurrentStage() once the blocked JNI call finally
                        // unwinds — long after the watchdog marked the run,
                        // possibly into a NEWER run's state. Rethrow instead.
                        throw e
                    } catch (e: CoreException) {
                        ui.fail("engine error ${Err.name(e.code)}: ${e.message}")
                    } catch (e: Exception) {
                        ui.fail("unexpected error: ${e.message}")
                    }
                }
                try {
                    val finished = withTimeoutOrNull(DEMO_TIMEOUT_MS) { pipeline.join() }
                    if (finished == null) {
                        pipeline.cancel() // best-effort — see the watchdog note above
                        failCurrentStage(
                            "demo timed out after ${DEMO_TIMEOUT_MS / 1000}s " +
                                "(best-effort timeout: any blocked engine call was NOT interrupted — it leaks with the process; the run is marked failed, a fresh run uses fresh temp dirs)"
                        )
                    } else {
                        _state.update {
                            it.copy(runElapsedMs = (System.nanoTime() - started) / 1_000_000)
                        }
                    }
                } catch (e: CancellationException) {
                    // scope/user cancellation — not our watchdog (the watchdog
                    // surfaces as withTimeoutOrNull returning null).
                    pipeline.cancel()
                    failCurrentStage("cancelled")
                    throw e
                } finally {
                    _state.update { it.copy(running = false) }
                }
            }
        }
    }

    /* ---- console + stage helpers (run-generation guarded) ---- */

    /**
     * Run generation (review finding): a zombie pipeline — whose blocked JNI
     * call was parked in the engine when the watchdog timed the run out —
     * may resume AFTER a newer run has taken over the state machine. Every
     * state write from pipeline code goes through a per-run [RunUI] handle:
     * writes apply only while the handle's generation is still the current
     * one; stale writes are dropped silently.
     */
    private val runGeneration = AtomicLong(0)

    /** Bumped once per run (owner: runDemo; seam: unit tests). */
    internal fun nextGeneration(): Long = runGeneration.incrementAndGet()

    /** Per-run UI write handle for `gen` (seam for unit tests). */
    internal fun uiFor(gen: Long) = RunUI(gen)

    /**
     * The only state-machine writer pipeline code may use. All writes are
     * generation-checked; `fail` reuses [failCurrentStage] under the guard.
     */
    internal inner class RunUI(private val gen: Long) {
        fun update(transform: (DemoState) -> DemoState) {
            if (gen == runGeneration.get()) _state.update(transform)
        }

        fun log(key: String, value: String) {
            val line = "$key:  $value"
            update { it.copy(console = (it.console + line).takeLast(MAX_CONSOLE_LINES)) }
        }

        fun beginStage(stage: Stage) = update { st ->
            st.copy(stages = st.stages.map {
                if (it.stage == stage) it.copy(status = StageStatus.RUNNING) else it
            })
        }

        fun endStage(stage: Stage, detail: String) = update { st ->
            st.copy(stages = st.stages.map {
                if (it.stage == stage) it.copy(status = StageStatus.DONE, detail = detail) else it
            })
        }

        fun fail(detail: String) {
            if (gen == runGeneration.get()) failCurrentStage(detail)
        }
    }

    /** RunDemo-owned writes (watchdog/timeout): made while this run IS the
     *  current generation — the held mutex guarantees no newer run exists. */
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

    private suspend fun runPipeline(ui: RunUI) {
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
        // Destination-side RECV bytes up to action-ready (review P2: the total
        // above sums BOTH peers' counters — the source's serve-side recv is not
        // "bytes the destination needed to reach ready"). Each dest wire is
        // added exactly once: at its round-close, or at import completion.
        val destRecvReady = AtomicLong(0)

        // For fail-fast cleanup (see the catch below): the wire address of the
        // listener, hoisted out of the coroutineScope body.
        var drainAddr: String? = null

        try {
            // coroutineScope: the serve loop is a STRUCTURED child — a
            // pipeline failure/timeout cancels it (its blocked JNI threads
            // drain when the peer disconnects; handles stay leaked per the rules).
            coroutineScope {
            /* ---- 1 OPEN ---- */
            ui.beginStage(Stage.OPEN)
            val coreSrc = withContext(Dispatchers.IO) { Core.open(srcDir.absolutePath) }
            srcCore = coreSrc
            val coreDst = withContext(Dispatchers.IO) { Core.open(dstDir.absolutePath) }
            dstCore = coreDst
            val abi = abiVersion()
            ui.log("abi", "engine ${abi.major}.${abi.minor} (expect 1.2)")
            ui.log("core.src", "opened ${srcDir.absolutePath}")
            ui.log("core.dst", "opened ${dstDir.absolutePath}")
            coreSrc.setForeground(true)
            coreDst.setForeground(true)
            ui.log("foreground", "true on both cores (§20.2 admission)")
            if (toggles.chunkSize > 0) {
                // The engine admits chunk sizes in [4096, 8388608] (§18.6);
                // clamp the demo knob into the admitted range instead of
                // failing the run (device-run bug 2026-11-07: the old 256 B
                // toggle died at OPEN with E_TRANSFER_QUOTA "chunk size 256
                // outside [4096, 8388608]"). Parity with DemoPipeline.swift's
                // clamp.
                val chunk = clampChunkSize(toggles.chunkSize)
                coreSrc.setChunkSize(chunk)
                coreDst.setChunkSize(chunk)
                ui.log("chunk_size", "$chunk B (progressive transfer visible)")
            }
            if (toggles.budgetRejection) {
                coreDst.setBudget(TINY_BUDGET_JSON)
                ui.log("budget.dst", "TINY net budget set (§6.8 partial/refusal expected, then replenished)")
            }
            if (toggles.recoveryReport) {
                listOf("src" to coreSrc, "dst" to coreDst).forEach { (name, core) ->
                    val r = withContext(Dispatchers.IO) { core.recoveryReport() }
                    ui.log(
                        "recovery.$name",
                        "interrupted=${r.interruptedTransfers.size} hidden=${r.hiddenIncompleteObjects.size}" +
                            " ambiguous=${r.ambiguousSessions.size} trunc=${r.journalTruncated} clean=${r.clean}"
                    )
                }
            }
            ui.endStage(Stage.OPEN, "2 cores open (temp dirs)")

            /* ---- 2 IDENTITY ---- */
            ui.beginStage(Stage.IDENTITY)
            val idSrc = withContext(Dispatchers.IO) { Identity.generate("carryon-src") }
            srcId = idSrc
            val idDst = withContext(Dispatchers.IO) { Identity.generate("carryon-dst") }
            dstId = idDst
            val srcPin = withContext(Dispatchers.IO) { idSrc.pinHex() }
            val dstPin = withContext(Dispatchers.IO) { idDst.pinHex() }
            ui.log("pin.src", shortPin(srcPin))
            ui.log("pin.dst", shortPin(dstPin))
            ui.log("key.der", "SECRET — never logged/displayed (§22.3, EVD-006)")
            ui.update {
                it.copy(
                    sourcePin = shortPin(srcPin),
                    destPin = shortPin(dstPin),
                    sourcePinHex = srcPin,
                    destPinHex = dstPin,
                )
            }
            ui.endStage(Stage.IDENTITY, "2 identities, pins pinned")

            /* ---- 3 PAIR ---- */
            ui.beginStage(Stage.PAIR)
            // True UTC (review finding: the old SimpleDateFormat appended a
            // literal 'Z' to LOCAL wall-clock time — the pairing record was
            // mislabeled as UTC). minSdk 29 → java.time is available.
            val utc = java.time.format.DateTimeFormatter.ISO_INSTANT
                .format(java.time.Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS))
            val pairing = withContext(Dispatchers.IO) { pairDevices(idSrc, idDst, utc) }
            val trustSrc = withContext(Dispatchers.IO) { Trust.fromJson(pairing.localTrustJson) }
            srcTrust = trustSrc
            val trustDst = withContext(Dispatchers.IO) { Trust.fromJson(pairing.remoteTrustJson) }
            dstTrust = trustDst
            ui.log("pairing", "mutual pin at $utc (trust_generation=0)")
            ui.endStage(Stage.PAIR, "mutual pin pairing")

            /* ---- 4 SESSION ---- */
            ui.beginStage(Stage.SESSION)
            val infoSrc = withContext(Dispatchers.IO) {
                coreSrc.registerAdapter(scenario.adapterId, scenario.adapterParams)
            }
            withContext(Dispatchers.IO) {
                coreDst.registerAdapter(scenario.adapterId, scenario.adapterParams)
            }
            ui.log("adapter", "${scenario.adapterId} registered on both cores")
            ui.log("adapter.info", infoSrc.take(120))
            val createReq = JSONObject().apply {
                put("adapter_id", scenario.adapterId)
                put("title", scenario.sessionTitle)
                put("privacy", "personal")
                put("authority_mode", "single_writer")
            }
            val sessionId = withContext(Dispatchers.IO) { coreSrc.createSession(createReq.toString()) }
            val cut = withContext(Dispatchers.IO) { coreSrc.createCut(sessionId) }
            ui.log("session.src", sessionId)
            ui.log("cut", "sealed number=${cut.number} (two-phase publish, CORE-004)")
            ui.endStage(Stage.SESSION, "session + cut ${cut.number}")

            /* ---- 5 TRANSFER ---- */
            ui.beginStage(Stage.TRANSFER)
            val lis = withContext(Dispatchers.IO) { Listener.bind("127.0.0.1:0") }
            listener = lis
            val addr = withContext(Dispatchers.IO) { lis.addr() }
            drainAddr = addr
            ui.log("listener", "bound $addr (TLS 1.3 loopback)")
            ui.update { it.copy(listenerAddr = addr) }

            // Wire rounds are deterministic given the toggles (see file header):
            // suspend -> +1, budget rejection -> +1.
            val expectedRounds =
                1 + (if (toggles.suspendResume) 1 else 0) + (if (toggles.budgetRejection) 1 else 0)
            var serveLoopError: Throwable? = null

            // SOURCE: bounded serve loop. Final round: serve_cut, then the same
            // session carries serve_authority_transfer (l3 flow; the dest's
            // request below pairs with it). EDITOR_L3 only: org.carryon.graph
            // declares supports_authority_transfer:false — for GRAPH the final
            // round is serve_cut alone (mirrors DemoPipeline.swift).
            val serveLoop = launch(Dispatchers.IO) {
                try {
                    for (round in 1..expectedRounds) {
                        val sess = Session.accept(lis, idSrc, trustSrc)
                        sess.serverNegotiate()
                        try {
                            coreSrc.serveCut(sess)
                            ui.log("serve.round$round", "cut served")
                        } catch (e: CoreException) {
                            // Device-run bug 2026-11-07: a mid-serve wire close
                            // on a NON-final round is the EXPECTED §11.2
                            // suspend / §6.8 refusal signal — the dest
                            // disconnects and reconnects fresh (the reference
                            // server, round_trip.rs, IGNORES serve_cut's
                            // result entirely and accepts the next round).
                            // Fatal only on the FINAL round (the completed
                            // import / authority wire) and on
                            // accept/negotiate failures, as before.
                            if (round == expectedRounds) throw e
                            ui.log(
                                "serve.round$round",
                                "wire closed mid-serve (${Err.name(e.code)}) — expected for suspend/§6.8 rounds; awaiting reconnect"
                            )
                        }
                        if (round == expectedRounds && scenario == Scenario.EDITOR_L3) {
                            coreSrc.serveAuthorityTransfer(sess, sessionId, cut.number)
                            ui.log("serve.authority", "relinquished; source read-only (§21.2)")
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
                    ui.log("serve", "serve loop failed: ${e.message}")
                }
            }

            // DESTINATION: connect -> negotiate -> import, honoring the toggles.
            val tImport0 = System.nanoTime()
            var wire: Session? = null
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

                // Counter discipline (see file header): NO live polling of
                // session_bytes while a blocking JNI call runs — the handle is
                // not aliased during the borrow (carryon.h). Counters are
                // sampled only after each wire operation completes, below.

                // Arm the §11.2 suspend so it fires before the FIRST chunk of
                // the import attempt it belongs to — but NOT when the §6.8
                // budget-refusal round is still ahead: an armed suspend
                // preempts the budget refusal (suspend hits before any bytes
                // move), which collapses the two showcases into one wire and
                // breaks the deterministic round accounting (device-run bug
                // 2026-11-07; see the file header's EXACT ROUND ORDER).
                if (toggles.suspendResume && !suspendRequested &&
                    !(toggles.budgetRejection && !budgetReplenished)
                ) {
                    withContext(Dispatchers.IO) { coreDst.requestSuspend() }
                    suspendRequested = true
                    ui.log("suspend", "armed before the import's first chunk (§11.2)")
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
                        // Sample between ops only: import_cut returned, no
                        // other coroutine touches this handle (the source's
                        // session is its own handle). Displayed WITHOUT
                        // tallying into the totals — the full wire (import +
                        // authority, or import for GRAPH) is tallied once at
                        // its close below. The dest recv of THIS wire counts
                        // toward action-ready (this is the state boundary).
                        val (bs, br) = sess.bytes()
                        destRecvReady.addAndGet(br)
                        ui.update {
                            it.copy(
                                bytesSent = totalSent.get() + bs,
                                bytesRecv = totalRecv.get() + br,
                            )
                        }
                        ui.log("import", "completed (mirror_cut=${o.mirrorCut})")
                    }
                    is ImportOutcome.Suspended -> {
                        ui.log(
                            "import",
                            "SUSPENDED mid-object, resume_token issued (interrupted ≠ success, §19.3.9)"
                        )
                        resumeToken = o.resumeTokenJson
                        outcome = null
                    }
                    is ImportOutcome.PartialBudget -> {
                        ui.log(
                            "import",
                            "PARTIAL-BUDGET prefix: published=${o.published}" +
                                " deferred=${o.deferred} (${o.deferredBytes} B) — §6.8"
                        )
                        outcome = null
                    }
                    null -> {
                        ui.log("budget", "import REFUSED (E_BUDGET_NETWORK) — normal outcome, §6.8")
                    }
                }

                if (outcome == null) {
                    // This round's wire is done: tally, close.
                    // (Suspended/partial/refused wires are closed by protocol;
                    // the resume/retry reconnects fresh.)
                    val (bs, br) = sess.bytes()
                    totalSent.addAndGet(bs)
                    totalRecv.addAndGet(br)
                    destRecvReady.addAndGet(br) // dest-side bytes on this wire count toward ready
                    ui.update {
                        it.copy(bytesSent = totalSent.get(), bytesRecv = totalRecv.get())
                    }
                    sess.close()
                    if (wire === sess) wire = null
                    if (toggles.budgetRejection && !budgetReplenished) {
                        withContext(Dispatchers.IO) { coreDst.setBudget(GENEROUS_BUDGET_JSON) }
                        budgetReplenished = true
                        ui.log("budget", "replenished; retrying (published objects skipped by dedup)")
                    }
                }
            }
            val tActionReady = System.nanoTime()
            val ttaMs = (tActionReady - tImport0) / 1e6
            ui.endStage(Stage.TRANSFER, "mirror cut ${cut.number} imported")

            /* ---- 6 AUTHORITY (same wire as the completed import — l3 flow) ---- */
            ui.beginStage(Stage.AUTHORITY)
            val mirror = withContext(Dispatchers.IO) { mirrorSessionId(sessionId) }
            ui.log("mirror_session", mirror)
            val cutRef = JSONObject().apply {
                put("session", mirror)
                put("number", cut.number)
            }
            // Read-only oracle first: the SAME logical session restored (l3 flow).
            // restore_latency_ms is the MEASURED duration of session.restore
            // itself (review P2: it used to reuse timeToActionReady — the
            // import duration, not the restore op). 0.0 when the op never runs.
            var restoreAgreed = false
            var restoreLatencyMs = 0.0
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
                restoreLatencyMs = (System.nanoTime() - tRestore0) / 1e6
                ui.log(
                    "session.restore",
                    "oracle agreed=$restoreAgreed " +
                        "(${String.format(Locale.US, "%.1f ms", restoreLatencyMs)})"
                )
            }
            // Err.ERR_BAD_HANDLE (-5): FFI-local — the import wire is gone.
            val authSess = wire ?: throw CoreException(Err.ERR_BAD_HANDLE, "import wire closed")
            var srcIndependentMs = 0.0
            if (scenario == Scenario.EDITOR_L3) {
                val preMutate = withContext(Dispatchers.IO) { coreDst.mayMutate(mirror) }
                ui.log("may_mutate.dst.pre", preMutate.toString())
                if (preMutate) {
                    throw CoreException(Err.E_AUTH_BASE + 2, "mirror must not start authoritative")
                }

                val tAuth0 = System.nanoTime()
                // The source is inside serve_authority_transfer (final serve round);
                // this request drives it. Durable receipts BEFORE the epoch moves
                // (AUTH-004).
                val receipt = withContext(Dispatchers.IO) {
                    coreDst.requestAuthorityTransfer(authSess, mirror, scenario.adapterId)
                }
                val epoch = JSONObject(receipt).optLong("new_epoch", -1)
                srcIndependentMs = (System.nanoTime() - tAuth0) / 1e6
                ui.log("authority", "receipt epoch=$epoch")
                ui.update { it.copy(authorityEpoch = epoch) }

                // Close the wire; the source's serve_authority_transfer returns,
                // tallies its counters and exits the serve loop.
                val (finalSent, finalRecv) = authSess.bytes()
                totalSent.addAndGet(finalSent)
                totalRecv.addAndGet(finalRecv)
                authSess.close()
                wire = null
                ui.update {
                    it.copy(bytesSent = totalSent.get(), bytesRecv = totalRecv.get())
                }
                serveLoop.join()
                serveLoopError?.let { throw it }
                ui.log("bytes", "sent=${totalSent.get()} recv=${totalRecv.get()} (MEASURED, §5.6)")
                ui.log("time_to_action_ready", String.format(Locale.US, "%.1f ms", ttaMs))

                val postDst = withContext(Dispatchers.IO) { coreDst.mayMutate(mirror) }
                val postSrc = withContext(Dispatchers.IO) { coreSrc.mayMutate(sessionId) }
                ui.log("may_mutate.dst.post", postDst.toString())
                ui.log("may_mutate.src.post", postSrc.toString())
                if (!postDst || postSrc) {
                    throw CoreException(Err.E_AUTH_BASE, "single-writer flip violated (§21.2)")
                }
                ui.update {
                    it.copy(destMayMutate = postDst, sourceMayMutate = postSrc)
                }
                ui.endStage(Stage.AUTHORITY, "epoch=$epoch · dest owner · src read-only")
            } else {
                // Honest skip (review finding: this used to call
                // requestAuthorityTransfer unconditionally, but
                // org.carryon.graph declares supports_authority_transfer:false —
                // the engine would refuse; the graph demo died at stage 6).
                // Mirrors DemoPipeline.swift's honest graph skip.
                val (finalSent, finalRecv) = authSess.bytes()
                totalSent.addAndGet(finalSent)
                totalRecv.addAndGet(finalRecv)
                authSess.close()
                wire = null
                ui.update {
                    it.copy(bytesSent = totalSent.get(), bytesRecv = totalRecv.get())
                }
                serveLoop.join()
                serveLoopError?.let { throw it }
                ui.log("authority", "skipped — org.carryon.graph declares supports_authority_transfer:false (L4 is the editor flagship)")
                ui.log("bytes", "sent=${totalSent.get()} recv=${totalRecv.get()} (MEASURED, §5.6)")
                ui.log("time_to_action_ready", String.format(Locale.US, "%.1f ms", ttaMs))
                // No authority moved: the dest stays a read-only mirror for its
                // read-only computation (graph.shortest_path).
                ui.update {
                    it.copy(destMayMutate = false, sourceMayMutate = true)
                }
                ui.endStage(Stage.AUTHORITY, "skipped (adapter capability: no authority transfer)")
            }

            // Source-off = the SOURCE ENGINE closes too, not just the transport
            // (review finding: the engine context used to outlive the
            // disconnect until evidence export). Receipts are durable (§21.2);
            // the destination continues alone below.
            runCatching { srcCore?.close() }
            srcCore = null
            ui.log("core.src", "closed — source-off (dest continues alone)")

            /* ---- 7 EDIT (source-off independent continuation) ---- */
            ui.beginStage(Stage.EDIT)
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
            ui.log(
                "edit",
                "${editReq.getString("class")} oracle agreed=$editAgreed " +
                    "(${String.format(Locale.US, "%.1f ms", editMs)})"
            )
            if (!editAgreed) {
                throw CoreException(Err.E_ACTION_BASE + 3, "independent-edit oracle disagreed")
            }
            ui.endStage(Stage.EDIT, "source-off edit agreed")

            /* ---- 8 EVIDENCE ---- */
            ui.beginStage(Stage.EVIDENCE)
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
                // Destination-side recv only, up to action-ready (review P2:
                // was totalRecv, which also sums the source's serve-side
                // counters).
                put("bytes_recv_to_action_ready", destRecvReady.get())
                put("time_to_action_ready_ms", ttaMs)
                // MEASURED session.restore duration (review P2: was
                // timeToActionReadyMs — the import duration, not the op).
                put("restore_latency_ms", restoreLatencyMs)
                // EDITOR: the authority-transfer duration; GRAPH: no authority
                // moved, so the dest's own edit duration is the independence
                // measure (same choice as DemoPipeline.swift).
                put("source_independence_ms", if (scenario == Scenario.EDITOR_L3) srcIndependentMs else editMs)
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
            ui.log("evidence", "exported ${bundle.length} B -> ${bundleFile.name}")
            val report = withContext(Dispatchers.IO) {
                coreDst.verifyEvidence(bundleFile.absolutePath)
            }
            val ok = JSONObject(report).optBoolean("ok", false)
            ui.log("verify_evidence", report.take(160))
            if (!ok) {
                throw CoreException(Err.E_INTERNAL_BASE, "evidence verification did not report ok")
            }
            ui.endStage(Stage.EVIDENCE, "verified ok=true")
            ui.log("result", "pipeline complete — OBSERVED (LOCAL), spec §2/§30")

            ui.update {
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
        } catch (t: Throwable) {
            // Fail fast instead of burning the 120 s watchdog (device-run
            // finding 2026-11-07): a dest-side exception unwinds
            // coroutineScope, whose implicit join WAITS for the structured
            // serve-loop child — but that child is parked in a BLOCKING
            // accept() it cannot be cancelled out of (freeing the listener
            // under a blocked call is use-after-free), so every such failure
            // surfaced only as a watchdog timeout, hiding the real error.
            // One throwaway connection makes the parked accept() return; its
            // negotiate then fails against the closed wire and the serve loop
            // exits, draining the thread. Handles are still leaked per the
            // documented rules — this only un-wedges the FAILING run's
            // unwind so the true error reaches the UI immediately.
            val addr = drainAddr
            val identity = dstId
            val trust = dstTrust
            if (addr != null && identity != null && trust != null && t !is CancellationException) {
                runCatching {
                    withContext(Dispatchers.IO) {
                        val spare = Session.connect(addr, identity, trust)
                        spare.close()
                    }
                }
            }
            throw t
        } finally {
            // Failure path: handles held by blocked JNI calls are deliberately
            // leaked (freeing under a blocked call is use-after-free). Temp dirs
            // live in cacheDir; the process owns them.
        }
    }

    private fun shortPin(pinHex: String): String = pinHex.take(16) + "…"

    companion object {
        private const val MAX_CONSOLE_LINES = 400
        private const val DEMO_TIMEOUT_MS = 120_000L
        private const val MAX_WIRE_ATTEMPTS = 5

        /** §18.6: the engine admits chunk sizes in [4096, 8388608] (see
         *  carryon-core MIN/MAX_CHUNK_SIZE). The UI knob is clamped into the
         *  admitted range instead of failing the run — device-run bug
         *  2026-11-07: a 256 B toggle died at OPEN with E_TRANSFER_QUOTA
         *  "chunk size 256 outside [4096, 8388608]". Mirrors DemoPipeline.swift. */
        const val CHUNK_MIN_BYTES = 4096L
        const val CHUNK_MAX_BYTES = 8_388_608L

        /** Clamp the demo's chunk-size knob into the engine's admitted range. */
        fun clampChunkSize(requested: Long): Long =
            requested.coerceIn(CHUNK_MIN_BYTES, CHUNK_MAX_BYTES)

        /** §6.8 budget JSON (model/budget.rs serde shape) — macOS-parity tiny
         *  budget (DemoPipeline.swift: total_net_bytes=128). Device-run bug
         *  2026-11-07: this used to be 256, which is ABOVE the editor L3
         *  authoritative closure (59+55+83 = 197 B), so the import sailed
         *  through with NO refusal/partial — the §6.8 showcase silently did
         *  nothing and the dest's round accounting desynced from the source's
         *  serve loop (120 s watchdog hang). 128 sits between the two-object
         *  prefix (59+55=114 fits) and the full closure → a VISIBLE
         *  partial_budget (§6.8: budget rejection SHALL be visible). */
        const val TINY_BUDGET_JSON =
            "{\"total_net_bytes\":128,\"avg_net_rate\":65536,\"burst_bytes\":1024," +
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
