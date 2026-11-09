/*
 * QrScan.kt — the pure half of camera QR pairing (issue #26, spec §18.4.2).
 *
 * QrDecoder turns a camera luminance (Y) plane into text with zxing core (pure
 * Java, the same library that renders the destination's QR). QrScanGate is a
 * cheap pre-filter for whatever the camera saw. Neither decides validity: the
 * ENGINE (pairingPayloadParse) is the only authority, and scanned text then
 * takes exactly the same parse + pinned connect as pasted text. Everything
 * here can only REJECT; nothing is accepted without the engine.
 *
 * Labels (§2): JVM-tested logic. The camera path itself is TARGET until a
 * device run is recorded.
 */
package com.carryon.app.pairing

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader

/** Outcome of screening raw scanned text before the engine sees it. */
sealed class QrScanResult {
    /** A plausible pairing payload. Still to be validated by the engine. */
    data class Candidate(val payload: String) : QrScanResult()
    data class Rejected(val why: String) : QrScanResult()
}

object QrScanGate {
    /** The engine-formatted payload scheme (carryon://pair?v=1&n=…&a=…&k=…&r=…). */
    const val PREFIX = "carryon://pair?"

    /** Far above a real payload (name + addr + 64-hex pin + code); caps hostile QRs. */
    const val MAX_PAYLOAD_CHARS = 1024

    fun screen(raw: String?): QrScanResult {
        val text = raw?.trim().orEmpty()
        if (text.isEmpty()) return QrScanResult.Rejected("nothing decoded")
        if (text.length > MAX_PAYLOAD_CHARS) {
            return QrScanResult.Rejected("payload too long (" + text.length + " chars)")
        }
        if (text.any { it.isISOControl() }) {
            return QrScanResult.Rejected("payload contains control characters")
        }
        if (!text.startsWith(PREFIX)) return QrScanResult.Rejected("not a Carry-On pairing code")
        return QrScanResult.Candidate(text)
    }
}

object QrDecoder {
    private val hints: Map<DecodeHintType, Any> = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true,
    )

    /**
     * Decodes one QR code from a luminance plane. [rowStride] must be at least
     * [width], and [data] must hold rowStride × height bytes. Returns null when
     * nothing decodes or the geometry is inconsistent; a bad frame never throws.
     */
    fun decodeLuminance(data: ByteArray, width: Int, height: Int, rowStride: Int = width): String? {
        if (width <= 0 || height <= 0 || rowStride < width) return null
        if (data.size.toLong() < rowStride.toLong() * height) return null
        return try {
            val source = PlanarYUVLuminanceSource(data, rowStride, height, 0, 0, width, height, false)
            QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source)), hints).text
        } catch (e: ReaderException) {
            null
        } catch (e: IllegalArgumentException) {
            null
        }
    }
}
