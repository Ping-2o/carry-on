/*
 * DiscoveryContractInstrumentedTest.kt — the ENGINE-owned §18.3 TXT contract
 * through the real engine .so ON DEVICE (stage 4): encode can only emit the
 * five minimal keys; validate round-trips, ignores foreign keys, and FAILS
 * CLOSED with E_PLATFORM_DISCOVERY (1105) on a malformed record — the app MUST
 * drop the peer, never repair it.
 *
 * Labels (§2): device-local contract tests — no network or transport claim
 * here (the physical mac⇄A04 discovery run is recorded separately in
 * docs/evidence/).
 */
package com.carryon.app.discovery

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.carryon.app.ffi.CoreException
import com.carryon.app.ffi.DiscoveryContract
import com.carryon.app.ffi.DiscoveryTxt
import com.carryon.app.ffi.Err
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DiscoveryContractInstrumentedTest {

    private fun txt() = DiscoveryTxt(
        protocolMajor = 1,
        instanceId = "ab12cd34",
        pairingRequired = true,
        deviceClass = "mobile",
        connectionHint = "54321",
    )

    @Test
    fun encodeEmitsExactlyTheFiveSection1833KeysInOrder() {
        val pairs = NsdTxtPairs.decodePairs(DiscoveryContract.encode(txt()))
        assertEquals(listOf("pm", "id", "pr", "dc", "ch"), pairs.map { it.first })
        assertEquals("1", pairs[0].second)
        assertEquals("mobile", pairs[3].second)
    }

    @Test
    fun encodeRefusesBadDeviceClassFailClosed() {
        try {
            DiscoveryContract.encode(txt().copy(deviceClass = "iPhone 14"))
            fail("a model string is not a coarse device class — engine must refuse")
        } catch (e: CoreException) {
            assertEquals(Err.E_PLATFORM_BASE + 5, e.code)
        }
    }

    @Test
    fun validateRoundTripsAndIgnoresForeignKeys() {
        val pairs = NsdTxtPairs.decodePairs(DiscoveryContract.encode(txt())) +
            listOf("vendor_key" to "opaque", "user" to "alice")
        val (validated, ignored) = DiscoveryContract.validate(pairs)
        assertEquals(1, validated.protocolMajor)
        assertEquals("ab12cd34", validated.instanceId)
        assertTrue(validated.pairingRequired)
        assertEquals("mobile", validated.deviceClass)
        assertEquals(setOf("vendor_key", "user"), ignored.toSet())
    }

    @Test
    fun validateFailsClosedOnMalformedRecord() {
        // The pilots' two pr encodings both parse; a malformed pm must refuse.
        val pairs = NsdTxtPairs.decodePairs(DiscoveryContract.encode(txt())).toMutableList()
        pairs[0] = "pm" to "one"
        try {
            DiscoveryContract.validate(pairs)
            fail("malformed TXT must fail closed — the peer is dropped, never repaired")
        } catch (e: CoreException) {
            assertEquals(Err.E_PLATFORM_BASE + 5, e.code)
        }
        // Missing required key: same refusal.
        try {
            DiscoveryContract.validate(pairs.drop(1))
            fail("missing key must fail closed")
        } catch (e: CoreException) {
            assertEquals(Err.E_PLATFORM_BASE + 5, e.code)
        }
    }
}
