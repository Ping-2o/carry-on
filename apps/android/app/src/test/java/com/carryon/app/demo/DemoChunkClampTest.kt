/*
 * DemoChunkClampTest.kt — regression for the 2026-11-07 device-run crash of the
 * chunk-size toggle (JVM unit: the clamp is pure logic; the FFI boundary itself
 * is covered by the instrumented FfiSmokeTest).
 *
 * The bug: the Android toggle set chunk_size=256 raw, and the engine — which
 * admits [4096, 8388608] (carryon-core MIN/MAX_CHUNK_SIZE, §18.6) — refused
 * the whole run at the OPEN stage with
 *
 *   E_TRANSFER_QUOTA: chunk size 256 outside [4096, 8388608]
 *
 * The macOS pipeline clamps the knob into the admitted range; the Android
 * runner now does the same (DemoRunner.clampChunkSize), and this test pins the
 * exact clamp contract so the knob can never again send an out-of-range value
 * into the native call.
 */
package com.carryon.app.demo

import org.junit.Assert.assertEquals
import org.junit.Test

class DemoChunkClampTest {

    @Test
    fun `the old buggy 256 B toggle value is clamped up into the admitted range`() {
        // Exactly the value the broken toggle used to send (device crash
        // 2026-11-07: E_TRANSFER_QUOTA "chunk size 256 outside [4096, 8388608]").
        assertEquals(4096L, DemoRunner.clampChunkSize(256L))
    }

    @Test
    fun `in-range values pass through unchanged`() {
        assertEquals(4096L, DemoRunner.clampChunkSize(4096L))
        assertEquals(16384L, DemoRunner.clampChunkSize(16384L))
        assertEquals(65536L, DemoRunner.clampChunkSize(65536L))
        assertEquals(8_388_608L, DemoRunner.clampChunkSize(8_388_608L))
    }

    @Test
    fun `values above the engine maximum clamp down`() {
        assertEquals(8_388_608L, DemoRunner.clampChunkSize(16_777_216L))
    }

    @Test
    fun `clamped bounds match the engine contract`() {
        // carryon-core MIN_CHUNK_SIZE / MAX_CHUNK_SIZE (§18.6). If the engine
        // contract ever changes, this is the line that must be updated
        // deliberately — not silently.
        assertEquals(4L * 1024, DemoRunner.CHUNK_MIN_BYTES)
        assertEquals(8L * 1024 * 1024, DemoRunner.CHUNK_MAX_BYTES)
    }
}
