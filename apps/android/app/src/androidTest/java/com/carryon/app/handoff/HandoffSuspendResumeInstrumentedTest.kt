/*
 * HandoffSuspendResumeInstrumentedTest.kt — the stage-6 crash-drill on real
 * hardware, in-process: a cooperative suspend mid-import (§11.2) checkpoints
 * the transfer at a chunk boundary, closes the wire, and the SAME receive
 * completes by resuming on a FRESH connection (the source serves a second
 * leg). The transfer is SAVED — this is the mechanism a device recall or
 * background kill WOULD use cooperatively (the non-cooperative kill -9 case
 * fails closed and is recorded in the physical-run bundle instead).
 *
 * Also asserts the stage-6 facts: legs=2 in the result + evidence metrics,
 * bytes_before_first_action measured, and the honest MULTI-LEG ladder.
 *
 * Claim label (§2 law): OBSERVED (LOCAL) — device-local, loopback TLS. The
 * PHYSICAL mac⇄A04 drill is recorded in docs/evidence/.
 */
package com.carryon.app.handoff

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class HandoffSuspendResumeInstrumentedTest {

    private fun context(): Context = InstrumentationRegistry.getInstrumentation().targetContext

    private val launcher = { bytes: ByteArray, fileName: String, mime: String ->
        val uri = L0L1Launcher.writeToMediaStore(context(), bytes, fileName, mime)
        uri.toString() to L0L1Launcher.launch(context(), uri, mime)
    }

    /** A chunky determinate body so the suspend request lands mid-object. */
    private fun makeChunkyFile(): File {
        val dir = File(context().cacheDir, "handoff-test/${System.nanoTime()}").apply { mkdirs() }
        val f = File(dir, "carryon-chunky.png")
        f.writeBytes(
            byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A) +
                ByteArray(256 * 1024) { (it % 251).toByte() }
        )
        return f
    }

    @Test
    fun loopback_cooperative_suspend_resumes_and_completes() = runBlocking {
        val srcDir = File(context().cacheDir, "handoff-sr/src-${System.nanoTime()}").apply { mkdirs() }
        val dstDir = File(context().cacheDir, "handoff-sr/dst-${System.nanoTime()}").apply { mkdirs() }
        val png = makeChunkyFile()

        val armed = HandoffFlow.arm(dstDir)
        armed.use { a ->
            val sourceEvents = mutableListOf<HandoffEvent>()
            val receiveEvents = mutableListOf<HandoffEvent>()
            // The source serves TWO legs: the suspended attempt + the resume.
            val sourceJob = launch(Dispatchers.IO) {
                HandoffFlow.serveFile(
                    dataDir = srcDir, file = png,
                    addr = "127.0.0.1:${a.wireAddr.substringAfterLast(':')}",
                    destPin = a.pinHex, identity = null, tamper = false,
                    serveLegs = 2,
                ) { sourceEvents += it }            }
            val result = HandoffFlow.receive(
                armed = a, sourcePin = sourcePinOf(sourceEvents, sourceJob), chunkSize = 4096,
                workDir = dstDir, launcher = launcher,
                suspendDrill = true,
            ) { receiveEvents += it }
            sourceJob.join()

            assertTrue("receive did not verify", result.ok)
            assertEquals("local", result.evidenceClass)
            // §11.2: exactly two legs (the checkpoint + the resume).
            assertEquals("legs", 2, result.legsUsed)
            // The resume SAVED the transfer: the object verified byte-for-byte.
            assertEquals(
                HandoffValidation.sha256Hex(png.readBytes()),
                result.declaredHash,
            )
            assertTrue(result.verifyOk)
            // bytes_before_first_action is measured (wire recv of both legs).
            assertTrue("bytes_before_first_action must be measured", result.bytesBeforeFirstAction > 0)
            // The ladder completed through the saved resume.
            val ladder = receiveEvents.filterIsInstance<HandoffEvent.Ladder>()
            assertTrue(ladder.contains(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.OK)))
            assertTrue(ladder.contains(HandoffEvent.Ladder(HandoffLadderStep.L1_OPEN, HandoffOutcome.OK)))
            // The receiver's own console retells the §11.2 checkpoint honestly.
            val lines = receiveEvents.filterIsInstance<HandoffEvent.Console>().map { it.line }
            assertTrue("console must mention the checkpoint", lines.any { it.contains("suspended") || it.contains("checkpoint") })
            assertTrue("leg count visible", lines.any { it.contains("legs=2") })
        }
    }

    /** Pull the source pin out of its events (the manual channel of the test). */
    private suspend fun sourcePinOf(events: List<HandoffEvent>, job: kotlinx.coroutines.Job): String {
        repeat(100) {
            events.filterIsInstance<HandoffEvent.ReceiverPin>().firstOrNull()?.let { return it.pinHex }
            kotlinx.coroutines.delay(50)
        }
        throw AssertionError("source never emitted its pin")
    }
}
