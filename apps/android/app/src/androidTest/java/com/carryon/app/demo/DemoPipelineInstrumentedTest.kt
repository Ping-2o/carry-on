/*
 * DemoPipelineInstrumentedTest.kt — end-to-end regression for the 2026-11-07
 * device-run deadlock of the full-toggle demo (instrumented: the path is the
 * real engine through the C ABI on a real device — a JVM test cannot load the
 * FFI or reproduce the wire-round interleaving; runs via
 * ./gradlew connectedDebugAndroidTest, needs one connected device).
 *
 * The bug: with suspend/resume + budget rejection BOTH on, the source's serve
 * loop died at the first mid-serve wire close (E_PROTO_FRAMING — which IS the
 * §11.2 suspend signal; the reference server, round_trip.rs, ignores serve_cut
 * errors), while its round accounting (3 rounds) also no longer matched the
 * dest's actual 2-round flow (the armed suspend preempted the budget refusal,
 * so the replenish at the suspend round's close removed the budget round).
 * Result: round 2 hung in accept() until the 120 s watchdog.
 *
 * This test runs the EXACT previously-hanging configuration — EDITOR_L3 with
 * chunk size + budget rejection + suspend/resume + recovery report — through
 * the real loopback TLS wire, and asserts the pipeline completes with every
 * stage DONE, verify ok, and the single-writer may_mutate flip. The watchdog
 * (DemoRunner) bounds it: a recurrence surfaces as st.error, so the test
 * cannot hang indefinitely.
 */
package com.carryon.app.demo

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DemoPipelineInstrumentedTest {

    private fun awaitDone(runner: DemoRunner, timeoutMs: Long = 150_000L) {
        // runDemo() only LAUNCHES the run; wait for it to actually start
        // (running=true) before waiting for completion — otherwise the first
        // poll can observe the pre-run state and pass vacuously.
        val started = System.currentTimeMillis() + 10_000L
        while (!runner.state.value.running && System.currentTimeMillis() < started) {
            Thread.sleep(50)
        }
        assertTrue("runDemo never started (running stayed false for 10s)", runner.state.value.running)
        val deadline = System.currentTimeMillis() + timeoutMs
        while (runner.state.value.running && System.currentTimeMillis() < deadline) {
            Thread.sleep(250)
        }
        assertFalse("runner still running after ${timeoutMs / 1000}s", runner.state.value.running)
    }

    private fun assertCompleted(runner: DemoRunner, expectAuthorityFlip: Boolean = true) {
        val st = runner.state.value
        // Console tail in the failure message: a hang/timeout's console is the
        // only witness of HOW FAR the pipeline got (device-run debugging).
        val consoleTail = st.console.takeLast(12).joinToString("\n")
        assertNull(
            "demo failed: ${st.error}\nconsole tail:\n$consoleTail",
            st.error,
        )
        st.stages.forEach { stageUi ->
            assertEquals("stage ${stageUi.stage}: ${stageUi.detail}", StageStatus.DONE, stageUi.status)
        }
        // Verify ok (stage 8's own check re-asserted at the assertion level).
        assertNotNull(st.verifyReportJson)
        assertTrue(
            "verify_evidence not ok: ${st.verifyReportJson}",
            JSONObject(st.verifyReportJson).optBoolean("ok", false),
        )
        if (expectAuthorityFlip) {
            // Single-writer flip (§21.2): dest owns, source read-only.
            assertEquals(java.lang.Boolean.FALSE, st.sourceMayMutate)
            assertEquals(java.lang.Boolean.TRUE, st.destMayMutate)
        }
        assertNotNull(st.evidenceBundleJson)
    }

    /** The exact configuration that hung on device before the round-order fix. */
    @Test
    fun editorL3WithAllTogglesCompletes() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val dir = File(context.cacheDir, "demo-it-${System.currentTimeMillis()}")
        val runner = DemoRunner(CoroutineScope(Dispatchers.Main.immediate), dir)
        runner.setScenario(Scenario.EDITOR_L3)
        runner.setToggles(
            DemoToggles(
                chunkSize = DemoRunner.CHUNK_MIN_BYTES, // 4 KiB toggle
                budgetRejection = true,                 // round 1: §6.8 refusal
                suspendResume = true,                   // round 2: §11.2 suspend
                recoveryReport = true,                  // recovery fields at OPEN
            )
        )
        runner.runDemo()
        awaitDone(runner)
        assertCompleted(runner)
        // The console must show BOTH showcase marks: the §6.8 refusal outcome
        // and the §11.2 suspension (interrupted ≠ success).
        val suspendLine = runner.state.value.console.firstOrNull { it.startsWith("import:") && it.contains("SUSPENDED") }
        assertNotNull("no SUSPENDED console line for the §11.2 round", suspendLine)
        val budgetMark = runner.state.value.console.firstOrNull {
            it.startsWith("budget:") && (it.contains("REFUSED") || it.contains("replenished"))
        }
        assertNotNull("no budget console mark for the §6.8 round", budgetMark)
    }

    /** Baseline: the no-toggle editor pipeline (the first-ever device smoke). */
    @Test
    fun editorL3BaselineCompletes() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val dir = File(context.cacheDir, "demo-it-${System.currentTimeMillis()}")
        val runner = DemoRunner(CoroutineScope(Dispatchers.Main.immediate), dir)
        runner.setScenario(Scenario.EDITOR_L3)
        runner.setToggles(DemoToggles())
        runner.runDemo()
        awaitDone(runner)
        assertCompleted(runner)
    }

    /** GRAPH with every toggle on: the §6.8 refusal round (graph objects exceed
     *  the tiny budget — a typed refusal, not a partial), then suspend, then
     *  resume; authority honestly SKIPPED (supports_authority_transfer:false),
     *  the mirror stays read-only and the dest runs graph.shortest_path. */
    @Test
    fun graphWithAllTogglesCompletes() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val dir = File(context.cacheDir, "demo-it-${System.currentTimeMillis()}")
        val runner = DemoRunner(CoroutineScope(Dispatchers.Main.immediate), dir)
        runner.setScenario(Scenario.GRAPH)
        runner.setToggles(
            DemoToggles(
                chunkSize = DemoRunner.CHUNK_MIN_BYTES,
                budgetRejection = true,
                suspendResume = true,
                recoveryReport = true,
            )
        )
        runner.runDemo()
        awaitDone(runner)
        assertCompleted(runner, expectAuthorityFlip = false)
        val st = runner.state.value
        // Honest skip: authority never moved for the graph scenario.
        assertTrue(
            "graph authority stage must record the honest skip, got: ${
                st.stages.first { it.stage == Stage.AUTHORITY }.detail
            }",
            st.stages.first { it.stage == Stage.AUTHORITY }.detail.contains("skipped"),
        )
        assertEquals(java.lang.Boolean.FALSE, st.destMayMutate) // read-only mirror
        assertEquals(java.lang.Boolean.TRUE, st.sourceMayMutate) // source keeps it
        // The §6.8 round for graph is a typed REFUSAL (objects > budget).
        assertTrue(
            "expected the §6.8 refusal console line for graph",
            runner.state.value.console.any { it.startsWith("budget:") },
        )
    }
}
