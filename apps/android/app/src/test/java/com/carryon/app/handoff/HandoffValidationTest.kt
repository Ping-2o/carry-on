/*
 * HandoffValidationTest.kt — JVM units for the L0/L1 handoff's pure logic:
 * §18.6 chunk clamp (the stage-1 device-bug class), manual-IP address
 * validation, pin validation, the §3.8-safe descriptor round-trip (fail closed
 * on malformed input, unknown features ignored), the CORE-004 integrity gate,
 * the mime map, and the evidence-class derivation.
 *
 * Pure JVM: proves the VALIDATION LOGIC, not any transport claim (labels
 * follow §2 — the pipeline + evidence bundles carry those).
 */
package com.carryon.app.handoff

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HandoffValidationTest {

    // ---- §18.6 chunk clamp (stage-1 bug regression class) ----

    @Test
    fun chunkClamp_bounds() {
        assertEquals(4096L, HandoffValidation.clampChunkSize(0L))
        assertEquals(4096L, HandoffValidation.clampChunkSize(256L)) // the stage-1 device bug value
        assertEquals(4096L, HandoffValidation.clampChunkSize(4095L))
        assertEquals(4096L, HandoffValidation.clampChunkSize(4096L))
        assertEquals(65536L, HandoffValidation.clampChunkSize(65536L))
        assertEquals(8_388_608L, HandoffValidation.clampChunkSize(8_388_608L))
        assertEquals(8_388_608L, HandoffValidation.clampChunkSize(Long.MAX_VALUE))
    }

    // ---- manual-IP receiver address validation ----

    @Test
    fun receiverAddr_accepts_strict_ipv4_port() {
        assertTrue(HandoffValidation.isValidReceiverAddr("192.168.3.50:48961"))
        assertTrue(HandoffValidation.isValidReceiverAddr("10.0.0.1:1"))
        assertTrue(HandoffValidation.isValidReceiverAddr("255.255.255.255:65535"))
        assertTrue(HandoffValidation.isValidReceiverAddr("127.0.0.1:48961")) // loopback → LOCAL class
    }

    @Test
    fun receiverAddr_rejects_malformed() {
        assertFalse(HandoffValidation.isValidReceiverAddr(""))
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50")) // no port
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:"))
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:0")) // port 0
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:65536"))
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:abc"))
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:007")) // leading zeros
        assertFalse(HandoffValidation.isValidReceiverAddr("256.168.3.50:48961"))
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3:48961"))
        assertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50.1:48961"))
        assertFalse(HandoffValidation.isValidReceiverAddr("a.b.c.d:80"))
        assertFalse(HandoffValidation.isValidReceiverAddr("host:80")) // manual mode is IPv4 until discovery
        assertFalse(HandoffValidation.isValidReceiverAddr("::1:48961"))
    }

    // ---- pin validation ----

    @Test
    fun pin_validation() {
        val good = "0123456789abcdef".repeat(4)
        assertTrue(HandoffValidation.isValidPinHex(good))
        assertFalse(HandoffValidation.isValidPinHex(good.uppercase())) // lowercase only, strict
        assertFalse(HandoffValidation.isValidPinHex("ab".repeat(31))) // 62
        assertFalse(HandoffValidation.isValidPinHex(good + "g"))
        assertFalse(HandoffValidation.isValidPinHex(""))
    }

    // ---- descriptor: §3.8-safe encode/parse ----

    private val hash = "ab".repeat(32)

    private fun descriptor() = HandoffDescriptor(
        session = "11111111-2222-3333-4444-555555555555", cut = 7,
        contentHash = hash, fileName = "carryon-doc.png",
        mimeType = "image/png", evidenceClass = "physical",
    )

    @Test
    fun descriptor_roundTrip() {
        assertEquals(descriptor(), HandoffDescriptor.parse(descriptor().features()))
    }

    @Test
    fun descriptor_failsClosed_onMalformed() {
        val d = descriptor()
        assertNull(HandoffDescriptor.parse(emptyList())) // no marker
        assertNull(HandoffDescriptor.parse(listOf(HandoffDescriptor.MARKER))) // marker only
        assertNull(HandoffDescriptor.parse(d.features().filterNot { it.startsWith("hash=") })) // missing hash
        assertNull(HandoffDescriptor.parse(d.features().map { if (it.startsWith("hash=")) "hash=zz" else it }))
        assertNull(HandoffDescriptor.parse(d.features().map { if (it.startsWith("evid=")) "evid=sure" else it }))
        // Unknown extra features are ignored, never interpreted (§18.3 style) —
        // a hostile probe must not change the parse.
        assertEquals(d, HandoffDescriptor.parse(d.features() + "exec=/bin/sh"))
    }

    // ---- CORE-004 integrity gate ----

    @Test
    fun integrityGate_declaredEqualsMeasured_passes() {
        val bytes = "carry-on file handoff bytes".toByteArray()
        val declared = HandoffValidation.sha256Hex(bytes)
        assertTrue(IntegrityGate.verify(bytes, declared))
    }

    @Test
    fun integrityGate_mismatch_refuses() {
        val bytes = "carry-on file handoff bytes".toByteArray()
        val declared = HandoffValidation.sha256Hex(bytes)
        // One flipped byte → refuse.
        assertFalse(IntegrityGate.verify(bytes + 0x00, declared))
        // A tampered DECLARED hash → refuse (the negative-test path).
        assertFalse(IntegrityGate.verify(bytes, declared.replace('a', 'b')))
        // Uppercase declared hash is accepted (case-insensitive digest compare).
        assertTrue(IntegrityGate.verify(bytes, declared.uppercase()))
    }

    // ---- mime map + evidence class ----

    @Test
    fun mime_map() {
        assertEquals("image/png", HandoffValidation.mimeForExtension("png"))
        assertEquals("image/png", HandoffValidation.mimeForExtension("PNG"))
        assertEquals("image/jpeg", HandoffValidation.mimeForExtension("jpg"))
        assertEquals("application/pdf", HandoffValidation.mimeForExtension("pdf"))
        assertEquals("text/plain", HandoffValidation.mimeForExtension("txt"))
        assertEquals("application/octet-stream", HandoffValidation.mimeForExtension("weird"))
        assertEquals("application/octet-stream", HandoffValidation.mimeForExtension(""))
    }

    @Test
    fun evidenceClass_loopback_vs_lan() {
        assertEquals("local", HandoffValidation.evidenceClass("127.0.0.1:48961"))
        assertEquals("physical", HandoffValidation.evidenceClass("192.168.3.50:48961"))
        assertEquals("physical", HandoffValidation.evidenceClass("10.1.2.3:1"))
    }

    @Test
    fun disclosure_honestPerClass() {
        val physical = ReceiveResult.disclosure("physical", "192.168.3.50:48961")
        assertTrue(physical.contains("PHYSICAL cross-device"))
        assertTrue(physical.contains("NOT a supported platform"))
        val local = ReceiveResult.disclosure("local", "127.0.0.1:48961")
        assertTrue(local.contains("LOCAL run on ONE DEVICE"))
        assertTrue(local.contains("NOT physical cross-device"))
    }
}
