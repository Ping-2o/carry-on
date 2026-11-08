/*
 * RecoveryReportParseTest.kt — regression for the 2026-11-07 device-run crash
 * of the "Recovery report" toggle (JVM unit: JSON parsing is pure logic; the
 * FFI boundary itself is covered by the instrumented FfiSmokeTest).
 *
 * The bug: the Kotlin wrapper typed the three list fields of
 * carryon_core_recovery_report_json as Int and called getInt on them, but the
 * engine (crates/carryon-core/src/recovery.rs + abi_core.rs) emits them as
 * JSON ARRAYS of ids:
 *
 *   org.json.JSONException: Value [] at interrupted_transfers of type
 *   org.json.JSONArray cannot be converted to int  →  the demo died at OPEN.
 *
 * The macOS decode (CarryonCore.swift) already modeled them as [String]; this
 * test pins the exact engine JSON shape — clean AND dirty — so a future ABI
 * transcription cannot silently reintroduce the mismatch.
 */
package com.carryon.app.kitcheck

import dev.carryon.kit.*

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RecoveryReportParseTest {

    /** The shape a fresh (clean) core emits — empty id arrays. */
    @Test
    fun `clean report parses with empty id lists`() {
        val json = """
            {"interrupted_transfers":[],"hidden_incomplete_objects":[],
             "ambiguous_sessions":[],"journal_truncated":false,"clean":true}
        """.trimIndent()
        val r = RecoveryReport.parse(json)
        assertTrue(r.clean)
        assertFalse(r.journalTruncated)
        assertEquals(emptyList<String>(), r.interruptedTransfers)
        assertEquals(emptyList<String>(), r.hiddenIncompleteObjects)
        assertEquals(emptyList<String>(), r.ambiguousSessions)
    }

    /** The shape a crashed core would emit — non-empty id arrays (§19.3:
     *  interrupted ≠ success; the ids are exactly what the UI must surface). */
    @Test
    fun `dirty report parses carrying the offending ids`() {
        val json = """
            {"interrupted_transfers":["t-1","t-2"],"hidden_incomplete_objects":["obj-a"],
             "ambiguous_sessions":["s-3"],"journal_truncated":true,"clean":false}
        """.trimIndent()
        val r = RecoveryReport.parse(json)
        assertFalse(r.clean)
        assertTrue(r.journalTruncated)
        assertEquals(listOf("t-1", "t-2"), r.interruptedTransfers)
        assertEquals(listOf("obj-a"), r.hiddenIncompleteObjects)
        assertEquals(listOf("s-3"), r.ambiguousSessions)
    }

    /** The old broken contract — int fields — must NOT parse: this pins that
     *  the engine really emits arrays (if the engine ever changes shape, this
     *  test fails loudly instead of the demo crashing on a device). */
    @Test(expected = org.json.JSONException::class)
    fun `an int-shaped report is NOT the engine contract`() {
        val json = """
            {"interrupted_transfers":0,"hidden_incomplete_objects":0,
             "ambiguous_sessions":0,"journal_truncated":false,"clean":true}
        """.trimIndent()
        RecoveryReport.parse(json)
    }
}
