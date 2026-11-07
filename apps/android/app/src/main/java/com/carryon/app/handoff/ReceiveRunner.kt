/*
 * ReceiveRunner.kt — the "Receive" screen's state machine (the Android sibling
 * of CarryFileModel): arms a receiver (core + identity + listener, shown to
 * the source BEFORE it serves), then runs the full L0/L1 receive when the
 * user accepts. State names follow §16.2 (Idle, Preparing, Importing,
 * ActionReady, Failed — never the bare word "Ready").
 *
 * Zombie isolation + watchdog (DemoRunner rules, simplified to one run):
 *  - one accept at a time (mutex);
 *  - the watchdog bounds the UI STATE MACHINE, not the native calls — a
 *    blocked JNI accept cannot be interrupted; on timeout the run is marked
 *    FAILED immediately and the parked accept + its handles LEAK with the
 *    process (freeing under a blocked call is use-after-free). A fresh arm
 *    uses a FRESH core/identity/listener, so a leaked wedge cannot poison it.
 *  - a zombie's late writes are dropped by run generation.
 */
package com.carryon.app.handoff

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.atomic.AtomicLong

/** UI phase of the receive screen (§16.2-aligned; plus the ladder's own run). */
data class ReceiveState(
    val armed: Boolean = false,
    /** The display address the source dials ("<site-local-ipv4>:<port>"). */
    val displayAddr: String = "",
    /** This receiver's FULL 64-hex pin (public fingerprint, copyable). */
    val pinHex: String = "",
    /** 16-hex truncated pin for the §22.3-style display line. */
    val shortPin: String = "",
    /** Exact §16.2 session state (never the bare word "Ready"). */
    val sessionState: String? = null,
    val ladder: Map<HandoffLadderStep, HandoffOutcome> = emptyMap(),
    val console: List<String> = emptyList(),
    val bytesSent: Long = 0,
    val bytesRecv: Long = 0,
    val running: Boolean = false,
    val lastResult: ReceiveResult? = null,
    val error: String? = null,
    val runElapsedMs: Long = 0,
) {
    companion object {
        /** The receive screen's honest static label; the per-run footer in the
         *  console is DYNAMIC (LOCAL vs PHYSICAL by the descriptor). */
        const val RUN_LABEL =
            "L0/L1 handoff — OBSERVED per run (LOCAL on loopback, PHYSICAL across two real devices); platform supported: NO (§30 unrun)"
    }
}

class ReceiveRunner(
    private val scope: CoroutineScope,
    private val context: Context,
    private val baseDir: File,
) {
    private val mutex = Mutex()
    private val _state = MutableStateFlow(ReceiveState())
    val state: StateFlow<ReceiveState> = _state.asStateFlow()

    private var armed: HandoffFlow.Armed? = null
    private val runGeneration = AtomicLong(0)

    /** Arm: open the core + identity + bind the listener; display addr + pin. */
    fun arm() {
        scope.launch {
            mutex.withLock {
                if (_state.value.running) return@withLock
                // A fresh arm per handoff: close the previous arm's handles if
                // it is not wedged (a wedged arm's handles leak by the rules).
                armed?.let { prev -> runCatching { prev.close() } }
                val gen = runGeneration.incrementAndGet()
                try {
                    val workDir = File(baseDir, "handoff/${stamp()}")
                    val a = HandoffFlow.arm(workDir)
                    armed = a
                    _state.value = ReceiveState(
                        armed = true,
                        displayAddr = a.displayAddr,
                        pinHex = a.pinHex,
                        shortPin = a.shortPin,
                        sessionState = "Idle",
                        console = listOf(
                            "armed:  listener ${a.wireAddr} (display ${a.displayAddr})",
                            "pin:  ${a.shortPin}  (full pin copyable above — the source pairs it, §18.4)",
                        ),
                    )
                } catch (e: Exception) {
                    _state.value = ReceiveState(error = "arm failed: ${e.message}", sessionState = "Failed")
                }
            }
        }
    }

    /** Accept + import + verify + open. `sourcePin` is the source's 64-hex pin;
     *  `chunkSize` 0 = engine default (destination-controlled §18.6). */
    fun accept(sourcePin: String, chunkSize: Long) {
        scope.launch {
            mutex.withLock {
                val a = armed ?: return@withLock
                if (_state.value.running) return@withLock
                if (!HandoffValidation.isValidPinHex(sourcePin)) {
                    _state.update { it.copy(error = "source pin must be the 64-hex fingerprint from the source screen") }
                    return@withLock
                }
                val started = System.nanoTime()
                val gen = runGeneration.incrementAndGet()
                _state.update {
                    it.copy(running = true, error = null, lastResult = null,
                            ladder = mapOf(HandoffLadderStep.L0_CARRY to HandoffOutcome.RUNNING),
                            sessionState = "Preparing", console = it.console + "accept:  waiting for the source (pinned)…")
                }
                val workDir = File(baseDir, "handoff/${stamp()}").apply { mkdirs() }
                val pipeline = launch {
                    try {
                        val result = HandoffFlow.receive(
                            armed = a, sourcePin = sourcePin, chunkSize = chunkSize,
                            workDir = workDir,
                            launcher = { bytes, fileName, mime ->
                                val uri = L0L1Launcher.writeToMediaStore(this@ReceiveRunner.context, bytes, fileName, mime)
                                uri.toString() to L0L1Launcher.launch(this@ReceiveRunner.context, uri, mime)
                            },
                            emitter = { event -> apply(gen, event) },
                        )
                        if (gen == runGeneration.get()) {
                            _state.update {
                                it.copy(
                                    running = false,
                                    lastResult = result,
                                    // §16.2 honesty: verify ok → ActionReady; a
                                    // bundle that does not verify is Inconclusive,
                                    // never a bare success.
                                    sessionState = if (result.ok) "ActionReady" else "Inconclusive",
                                    runElapsedMs = (System.nanoTime() - started) / 1_000_000,
                                )
                            }
                        }
                    } catch (e: kotlinx.coroutines.CancellationException) {
                        throw e
                    } catch (e: com.carryon.app.ffi.CoreException) {
                        fail(gen, "engine error ${com.carryon.app.ffi.Err.name(e.code)}: ${e.message}")
                    } catch (e: Exception) {
                        fail(gen, "receive failed: ${e.message}")
                    }
                }
                // Watchdog (DemoRunner rule): bound the UI state machine, not
                // the native calls. On timeout mark FAILED now; the blocked
                // accept + handles leak (fresh arm = fresh handles).
                val finished = withTimeoutOrNull(RECEIVE_TIMEOUT_MS) { pipeline.join() }
                if (finished == null) {
                    pipeline.cancel()
                    fail(gen, "receive timed out after ${RECEIVE_TIMEOUT_MS / 1000}s (the blocked engine call was NOT interrupted — it leaks with the process; re-arm for a fresh listener)")
                } else {
                    _state.update { it.copy(runElapsedMs = (System.nanoTime() - started) / 1_000_000) }
                }
            }
        }
    }

    private fun fail(gen: Long, why: String) {
        if (gen != runGeneration.get()) return // zombie write — drop
        _state.update {
            it.copy(
                running = false,
                error = why,
                sessionState = "Failed",
                console = (it.console + "FAIL:  $why").takeLast(MAX_CONSOLE_LINES),
            )
        }
    }

    private fun apply(gen: Long, event: HandoffEvent) {
        if (gen != runGeneration.get()) return // zombie write — drop
        _state.update { st ->
            when (event) {
                is HandoffEvent.Console -> st.copy(console = (st.console + event.line).takeLast(MAX_CONSOLE_LINES))
                is HandoffEvent.Ladder -> st.copy(ladder = st.ladder + mapOf(event.step to event.outcome))
                is HandoffEvent.State -> st.copy(sessionState = event.state)
                is HandoffEvent.Counters -> st.copy(bytesSent = event.sent, bytesRecv = event.recv)
                is HandoffEvent.ReceiverPin -> st.copy(pinHex = event.pinHex, shortPin = event.pinHex.take(16) + "…")
                is HandoffEvent.Finished -> st.copy(lastResult = event.result)
                is HandoffEvent.Failed -> st.copy(error = event.why)
            }
        }
    }

    private fun stamp(): String = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date())

    companion object {
        private const val MAX_CONSOLE_LINES = 400
        private const val RECEIVE_TIMEOUT_MS = 180_000L
    }
}
