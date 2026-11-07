/*
 * BudgetShowcaseTest.kt — regression for the 2026-11-07 device-run silent no-op
 * of the "Budget rejection" toggle (JVM unit: the budget knob is pure data;
 * the end-to-end path is covered by DemoPipelineInstrumentedTest).
 *
 * The bug: the toggle's tiny budget used total_net_bytes=256 — ABOVE the
 * editor L3 authoritative closure actually carried over the wire
 * (document 59 B + unsaved_edits 55 B + meta 83 B = 197 B; the optional
 * navigation object is not part of the authoritative cut manifest). The
 * import therefore sailed through with {"completed":true}: §6.8's REQUIRED
 * visible rejection/partial admission never happened, and the destination's
 * round accounting desynced from the source's bounded serve loop (120 s
 * watchdog hang). macOS uses 128 for exactly this reason ("each demo object
 * is < 208 bytes, so 128 forces a visible partial admission" —
 * DemoPipeline.swift).
 *
 * This test pins the invariant: the tiny budget MUST sit below the closure
 * cost so the showcase always bites. If the editor session_v1 contents ever
 * change, re-tune the constant deliberately.
 */
package com.carryon.app.demo

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class BudgetShowcaseTest {

    /** The editor L3 authoritative closure carried by the demo cut (bytes;
     *  org.json measures from the real adapter — see the class comment). */
    private val editorL3ClosureBytes = 59L + 55L + 83L

    @Test
    fun `tiny budget must sit BELOW the editor L3 closure so §68 bites`() {
        val budget = JSONObject(DemoRunner.TINY_BUDGET_JSON)
        val total = budget.getLong("total_net_bytes")
        assertTrue(
            "tiny budget $total must be < the editor L3 closure " +
                "($editorL3ClosureBytes B) — otherwise the §6.8 showcase " +
                "silently admits everything and the round accounting desyncs",
            total < editorL3ClosureBytes,
        )
        // macOS parity value (DemoPipeline.swift tinyBudgetBytes) — and the
        // two-object prefix (59+55=114) must still FIT so the admission is a
        // visible PARTIAL, not an instant refusal.
        assertEquals(128L, total)
        assertTrue(
            "the first two objects (document+unsaved, 114 B) must fit the " +
                "budget for a visible partial_budget prefix",
            59L + 55L <= total,
        )
    }

    /** The budget JSON must parse against model::Budget's full serde shape:
     *  a rename upstream surfaces here instead of as a runtime error. */
    @Test
    fun `tiny budget carries every model Budget field`() {
        val budget = JSONObject(DemoRunner.TINY_BUDGET_JSON)
        listOf(
            "total_net_bytes", "avg_net_rate", "burst_bytes", "total_cpu_ms",
            "cpu_duty", "peak_prep_mem", "storage_quota", "battery_thermal",
            "elapsed_opportunity_ms", "max_nonpreemptible_ms",
        ).forEach { key ->
            assertTrue("Budget field '$key' missing from TINY_BUDGET_JSON", budget.has(key))
        }
        assertEquals("Unrestricted", budget.getString("battery_thermal"))
    }
}
