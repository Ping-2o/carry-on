/*
 * HandoffLoopbackInstrumentedTest.kt — the FULL L0/L1 file handoff on real
 * hardware, in-process: source + destination cores on ONE device, loopback
 * TLS 1.3 (the DemoPipelineInstrumentedTest pattern). Drives the REAL app
 * code paths — HandoffFlow.serveFile + HandoffFlow.arm/receive + the real
 * MediaStore write + a resolvable ACTION_VIEW launch.
 *
 * Claim label (§2 law): OBSERVED (LOCAL) — DEVICE-LOCAL. One device, loopback
 * TLS. This is NOT physical cross-device transfer; the physical run (mac ⇄
 * this device over LAN) is recorded separately in the evidence bundle.
 */
package com.carryon.app.handoff

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class HandoffLoopbackInstrumentedTest {

    private fun context(): Context = InstrumentationRegistry.getInstrumentation().targetContext

    /** The real launcher the Receive screen uses (MediaStore + ACTION_VIEW). */
    private val launcher = { bytes: ByteArray, fileName: String, mime: String ->
        val uri = L0L1Launcher.writeToMediaStore(context(), bytes, fileName, mime)
        uri.toString() to L0L1Launcher.launch(context(), uri, mime)
    }

    private fun makePng(): File {
        val dir = File(context().cacheDir, "handoff-test/${System.nanoTime()}").apply { mkdirs() }
        val f = File(dir, "carryon-doc.png")
        // A real PNG header + deterministic body — small, honest test payload.
        f.writeBytes(
            byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A) +
                ByteArray(128) { (it % 251).toByte() }
        )
        return f
    }

    @Test
    fun loopback_positive_carry_verify_open() = runBlocking {
        val srcDir = File(context().cacheDir, "handoff-test/src-${System.nanoTime()}").apply { mkdirs() }
        val dstDir = File(context().cacheDir, "handoff-test/dst-${System.nanoTime()}").apply { mkdirs() }
        val png = makePng()

        // Destination armed first (the app's own arm() path): core + identity
        // + listener; its full pin goes to the source.
        val armed = HandoffFlow.arm(dstDir)
        armed.use { a ->
            assertTrue(HandoffValidation.isValidPinHex(a.pinHex))
            assertTrue(a.displayAddr.contains(":"))
            // The source must pin the RECEIVER — pairing needs both directions;
            // the receive pins the source, so here we pre-pair the receiver's
            // trust through receive()'s sourcePin param and pre-pair the
            // source's trust through serveFile's destPin (a.pinHex).
            // Separate event lists: the SOURCE's ladder events must never stand
            // in for the RECEIVER's own (the physical run's bug class).
            val sourceEvents = mutableListOf<HandoffEvent>()
            val receiveEvents = mutableListOf<HandoffEvent>()
            val sourceJob = launch(Dispatchers.IO) {
                HandoffFlow.serveFile(
                    dataDir = srcDir, file = png,
                    addr = "127.0.0.1:${a.wireAddr.substringAfterLast(':')}",
                    destPin = a.pinHex, identity = null, tamper = false,
                ) { sourceEvents += it }
            }
            val result = HandoffFlow.receive(
                armed = a, sourcePin = sourcePinOf(sourceEvents, sourceJob), chunkSize = 65536,
                workDir = dstDir, launcher = launcher,
            ) { receiveEvents += it }
            sourceJob.join()

            assertTrue("receive did not verify", result.ok)
            assertFalse(result.refused)
            assertEquals("local", result.evidenceClass) // loopback → honest LOCAL
            assertNotNull(result.verifiedBytes)
            // CORE-004 app-level gate: measured sha256 == declared, byte-for-byte.
            assertEquals(
                HandoffValidation.sha256Hex(png.readBytes()),
                HandoffValidation.sha256Hex(result.verifiedBytes!!.readBytes()),
            )
            assertEquals(result.declaredHash, HandoffValidation.sha256Hex(png.readBytes()))
            // The real OS launch resolved to a real activity (MediaStore path).
            assertNotNull("ACTION_VIEW did not resolve", result.resolvedActivity)
            assertNotNull(result.contentUri)
            assertTrue(result.verifyOk)
            // Ladder: both levels honest-done BY THE RECEIVER's own events.
            val ladder = receiveEvents.filterIsInstance<HandoffEvent.Ladder>()
            assertTrue(
                "receiver must emit L0 OK itself (source events don't count)",
                ladder.contains(HandoffEvent.Ladder(HandoffLadderStep.L0_CARRY, HandoffOutcome.OK)),
            )
            assertTrue(ladder.contains(HandoffEvent.Ladder(HandoffLadderStep.L1_OPEN, HandoffOutcome.OK)))
        }
    }

    @Test
    fun loopback_negative_tampered_hash_refused() = runBlocking {
        val srcDir = File(context().cacheDir, "handoff-test/src-${System.nanoTime()}").apply { mkdirs() }
        val dstDir = File(context().cacheDir, "handoff-test/dst-${System.nanoTime()}").apply { mkdirs() }
        val png = makePng()

        val armed = HandoffFlow.arm(dstDir)
        armed.use { a ->
            val sourceEvents = mutableListOf<HandoffEvent>()
            val receiveEvents = mutableListOf<HandoffEvent>()
            val sourceJob = launch(Dispatchers.IO) {
                // --tamper: the descriptor declares a TAMPERED hash; the
                // receive gate must REFUSE (fail closed) — no success on a
                // mismatched object (CORE-004).
                runCatching {
                    HandoffFlow.serveFile(
                        dataDir = srcDir, file = png,
                        addr = "127.0.0.1:${a.wireAddr.substringAfterLast(':')}",
                        destPin = a.pinHex, identity = null, tamper = true,
                    ) { sourceEvents += it }
                }
            }
            try {
                HandoffFlow.receive(
                    armed = a, sourcePin = sourcePinOf(sourceEvents, sourceJob), chunkSize = 0,
                    workDir = dstDir, launcher = launcher,
                ) { receiveEvents += it }
                org.junit.Assert.fail("receive must REFUSE a tampered declared hash")
            } catch (expected: dev.carryon.kit.CoreException) {
                // fail closed — the only correct outcome (CORE-004)
                assertTrue(expected.message!!.contains("REFUSED"))
            }
            sourceJob.join()
            // No L1 must ever have fired on the receiver.
            assertFalse(receiveEvents.any {
                it is HandoffEvent.Ladder && it.step == HandoffLadderStep.L1_OPEN &&
                    it.outcome == HandoffOutcome.OK
            })
        }
    }

    /** The source's pin: serveFile emits it once the engine identity exists. */
    private suspend fun sourcePinOf(
        events: MutableList<HandoffEvent>,
        job: kotlinx.coroutines.Job,
    ): String {
        for (i in 0..200) {
            events.filterIsInstance<HandoffEvent.ReceiverPin>()
                .lastOrNull()?.let { return it.pinHex }
            if (!job.isActive) break
            delay(50)
        }
        throw AssertionError("source pin never emitted")
    }
}
