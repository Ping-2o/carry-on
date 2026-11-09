/*
 * QrScanTest.kt — JVM tests for the pure half of camera QR pairing (#26):
 * zxing luminance decoding and the pre-engine gate. The engine parse (the
 * real validity authority) is exercised through the FFI elsewhere; a JVM
 * unit test cannot load the engine .so.
 *
 * Labels (§2): pure-logic tests; no camera or device claim.
 */
package com.carryon.app.pairing

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Random

class QrScanTest {

    private val sample =
        "carryon://pair?v=1&n=studio-mac&a=192.168.1.20:48961&k=" + "ab".repeat(32) + "&r=123456"

    /** Renders [text] as a QR luminance plane (0 = dark, 255 = light), rows padded to [rowStride]. */
    private fun render(text: String, size: Int = 240, rowStride: Int = size): ByteArray {
        val m = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size, mapOf(EncodeHintType.MARGIN to 4))
        val out = ByteArray(rowStride * size) { 0xFF.toByte() }
        for (y in 0 until size) {
            for (x in 0 until size) {
                if (m.get(x, y)) out[y * rowStride + x] = 0
            }
        }
        return out
    }

    @Test
    fun decodesARenderedPayload() {
        assertEquals(sample, QrDecoder.decodeLuminance(render(sample), 240, 240))
    }

    @Test
    fun decodesWithAPaddedRowStride() {
        // Camera Y planes often have rowStride > width.
        assertEquals(sample, QrDecoder.decodeLuminance(render(sample, rowStride = 256), 240, 240, rowStride = 256))
    }

    @Test
    fun noiseDecodesToNull() {
        val noise = ByteArray(240 * 240).also { Random(26).nextBytes(it) }
        assertNull(QrDecoder.decodeLuminance(noise, 240, 240))
    }

    @Test
    fun inconsistentGeometryIsNullNotACrash() {
        val buf = ByteArray(100)
        assertNull(QrDecoder.decodeLuminance(buf, 0, 10))
        assertNull(QrDecoder.decodeLuminance(buf, 20, 20))
        assertNull(QrDecoder.decodeLuminance(buf, 10, 10, rowStride = 5))
    }

    @Test
    fun gatePassesATrimmedCarryOnPayload() {
        assertEquals(QrScanResult.Candidate(sample), QrScanGate.screen("  " + sample + "\n"))
    }

    @Test
    fun gateRejectsEverythingElse() {
        val rejected = listOf(
            null,
            "",
            "   ",
            "123456",
            "https://example.com/pair?v=1",
            "CARRYON://PAIR?v=1",
            "carryon://pair?" + "a".repeat(QrScanGate.MAX_PAYLOAD_CHARS),
            "carryon://pair?v=1" + Char(7) + "&n=x",
        )
        for (raw in rejected) {
            assertTrue("should reject: " + raw, QrScanGate.screen(raw) is QrScanResult.Rejected)
        }
    }

    @Test
    fun gateDoesNotPretendToValidate() {
        // A well-prefixed but malformed payload is only a CANDIDATE: the engine
        // parse (pairingPayloadParse) is what refuses it, and PairingScreen
        // fills nothing when it does.
        assertTrue(QrScanGate.screen("carryon://pair?garbage") is QrScanResult.Candidate)
    }
}
