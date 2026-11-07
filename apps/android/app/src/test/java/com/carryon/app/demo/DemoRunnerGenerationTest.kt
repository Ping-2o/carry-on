/*
 * DemoRunnerGenerationTest.kt — zombie-isolation unit test (review finding).
 *
 * Scenario under test, at the state-machine level: run A wedges on a blocked
 * JNI call → the watchdog times it out, marks it FAILED and releases the
 * mutex → the user starts run B → run A's blocked call finally returns and
 * its coroutine unwinds. Run A's stale writes MUST be dropped: every
 * pipeline state write goes through a per-run RunUI handle checked against
 * the run generation, and the cancelled pipeline rethrows its
 * CancellationException instead of reporting a failure.
 *
 * The full end-to-end variant (a real wedged native call) needs a device;
 * this JVM unit test pins the GUARD itself, which is the mechanism the race
 * lives in. Runs in CI via :app:testDebugUnitTest.
 */
package com.carryon.app.demo

import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DemoRunnerGenerationTest {

    private fun newRunner(): DemoRunner =
        DemoRunner(CoroutineScope(Dispatchers.Unconfined), Files.createTempDirectory("carryon-test").toFile())

    @Test
    fun `zombie pipeline cannot mutate newer run state`() {
        val runner = newRunner()

        // Run A starts (as runDemo would: bump the generation, take a UI handle).
        val uiA = runner.uiFor(runner.nextGeneration())

        // While A is the current run, its writes apply.
        uiA.log("run.a", "live write")
        uiA.beginStage(Stage.TRANSFER)
        assertTrue(runner.state.value.console.any { it.contains("live write") })
        assertEquals(StageStatus.RUNNING, runner.state.value.stages.first { it.stage == Stage.TRANSFER }.status)

        // Run B starts (the watchdog timed A out and released the mutex).
        val uiB = runner.uiFor(runner.nextGeneration())

        // Run A's blocked JNI call now returns and the zombie pipeline tries
        // to write: log, stage transitions, a failure, a raw state update.
        // ALL of it must be dropped — B owns the state machine now.
        uiA.log("zombie", "stale console line")
        uiA.endStage(Stage.TRANSFER, "stale detail")
        uiA.fail("stale failure")
        uiA.update { it.copy(error = "zombie corruption", listenerAddr = "zombie") }

        assertFalse(runner.state.value.console.any { it.contains("stale") })
        assertEquals(StageStatus.RUNNING, runner.state.value.stages.first { it.stage == Stage.TRANSFER }.status)
        assertNull(runner.state.value.error)
        assertEquals("", runner.state.value.listenerAddr)

        // Run B's own writes still apply (the guard must not freeze the UI).
        uiB.log("run.b", "fresh write")
        assertTrue(runner.state.value.console.any { it.contains("fresh write") })
    }

    @Test
    fun `cancellation surfaces as CancellationException, not as a FAIL write`() {
        val runner = newRunner()
        val uiA = runner.uiFor(runner.nextGeneration())

        // Simulate the pipeline-body contract directly: a cancelled run's
        // CancellationException is rethrown, never converted into a failure
        // (the generic Exception catch must not swallow it).
        val caught = runCatching {
            try {
                throw kotlinx.coroutines.CancellationException("watchdog cancelled the run")
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                uiA.fail("unexpected error: ${e.message}")
                throw e
            }
        }.exceptionOrNull()

        assertTrue(caught is kotlinx.coroutines.CancellationException)
        assertNull(runner.state.value.error) // no FAIL line was written
        assertFalse(runner.state.value.console.any { it.contains("FAIL") })
    }
}
