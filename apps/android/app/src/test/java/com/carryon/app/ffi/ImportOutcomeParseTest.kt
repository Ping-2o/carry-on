/*
 * ImportOutcomeParseTest.kt — regression for the 2026-11-07 device-run hang of
 * the "Budget rejection" showcase (JVM unit: JSON parsing is pure logic; the
 * end-to-end path is covered by DemoPipelineInstrumentedTest).
 *
 * The bug: the Kotlin parse of carryon_import_cut's outcome called
 * getInt("deferred") on the `partial_budget` shape, but the engine
 * (abi_transfer.rs write_outcome) serializes `deferred` as a JSON ARRAY of
 * object ids. The org.json JSONException unwound the pipeline into the
 * structured serve loop parked in a blocking accept() — invisible until the
 * 120 s watchdog. The JSON below is VERBATIM the engine's output (captured on
 * the host against the real editor L3 adapter with total_net_bytes=128).
 */
package com.carryon.app.ffi

import org.junit.Assert.assertEquals
import org.junit.Test

class ImportOutcomeParseTest {

    /** Verbatim engine output (host probe, editor L3, tiny budget): note
     *  `deferred` is an ARRAY and `published` is an ARRAY of objects. */
    private val partialJson =
        """
        {"deferred":["editor.meta.v1"],"deferred_bytes":83,"partial_budget":true,
         "published":[{"generation":1,"object_id":"editor.document.v1"},
                      {"generation":1,"object_id":"editor.unsaved_edits.v1"}],
         "remote_cut":0,"remote_session":"94e43013-d593-49eb-9d1e-53a1139a53cd"}
        """.trimIndent()

    @Test
    fun `partial_budget parses with deferred as the id ARRAY the engine emits`() {
        val out = ImportOutcome.parse(partialJson)
        val p = out as ImportOutcome.PartialBudget
        assertEquals(2, p.published) // two published objects
        assertEquals(1, p.deferred) // ONE deferred object id in the array
        assertEquals(83L, p.deferredBytes)
        assertEquals("94e43013-d593-49eb-9d1e-53a1139a53cd", p.remoteSession)
        assertEquals(0L, p.remoteCut)
    }

    @Test
    fun `completed and suspended shapes still parse`() {
        assertEquals(
            ImportOutcome.Completed(mirrorCut = 0),
            ImportOutcome.parse("""{"completed":true,"mirror_cut":0}"""),
        )
        val suspended = ImportOutcome.parse(
            """{"suspended":true,"resume_token":{"session":"s","cut":0,"digest":"d"}}""",
        )
        assertEquals(
            ImportOutcome.Suspended::class.simpleName,
            suspended::class.simpleName,
        )
    }
}
