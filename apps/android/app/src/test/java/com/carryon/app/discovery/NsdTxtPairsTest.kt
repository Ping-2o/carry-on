/*
 * NsdTxtPairsTest.kt — JVM unit tests for the pure half of the Android
 * discovery bridge (stage 4): NSD attribute extraction and engine pairs-JSON
 * decoding. The CONTRACT half (encode/validate, fail-closed filtering) lives
 * in the engine and is exercised through the FFI in the instrumented tests
 * and on the Rust side — a JVM unit test cannot load the engine .so.
 *
 * Labels (§2): pure-logic tests, no network or transport claim.
 */
package com.carryon.app.discovery

import org.json.JSONArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NsdTxtPairsTest {

    @Test
    fun `null attribute values become empty-value pairs`() {
        // DNS-SD allows "=value-less" keys; NSD hands them back as null bytes.
        val pairs = NsdTxtPairs.fromAttributes(
            mapOf("pm" to "1".toByteArray(), "ch" to null)
        )
        assertEquals(listOf("pm" to "1", "ch" to ""), pairs)
    }

    @Test
    fun `decodePairs roundtrips the engine canonical form`() {
        val json = JSONArray()
            .put(JSONArray().put("pm").put("1"))
            .put(JSONArray().put("id").put("ab12cd34"))
            .put(JSONArray().put("pr").put("1"))
            .put(JSONArray().put("dc").put("mobile"))
            .put(JSONArray().put("ch").put("none"))
            .toString()
        val pairs = NsdTxtPairs.decodePairs(json)
        assertEquals(5, pairs.size)
        assertEquals("pm" to "1", pairs[0])
        assertEquals("ch" to "none", pairs[4])
        // §18.3 minimality: the decoded canonical form has exactly these keys.
        assertEquals(listOf("pm", "id", "pr", "dc", "ch"), pairs.map { it.first })
    }

    @Test
    fun `foreign keys survive extraction for the engine to ignore`() {
        // A hostile record carries foreign keys; extraction must preserve them
        // so the ENGINE (not this file) ignores them — fail-closed centralization.
        val pairs = NsdTxtPairs.fromAttributes(
            mapOf(
                "pm" to "1".toByteArray(),
                "id" to "ab12cd34".toByteArray(),
                "pr" to "true".toByteArray(),
                "dc" to "desktop".toByteArray(),
                "ch" to "49152".toByteArray(),
                "evil_key" to "never interpreted".toByteArray(),
            )
        )
        assertEquals(6, pairs.size)
        assertTrue(pairs.contains("evil_key" to "never interpreted"))
    }
}
