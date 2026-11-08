/*
 * KiwixMetricsTest.kt — JVM unit tests for the companion reader's pure logic
 * (stage 10, mobile half). The engine/FFI path is covered by the instrumented
 * KiwixSessionInstrumentedTest on a real device; here we pin the JSON shapes,
 * the escaping, and the flagship D/A math.
 */
package com.carryon.app.kiwix

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class KiwixMetricsTest {

    @Test
    fun `D over A ratio is the flagship metric in ppm`() {
        // The Phase-3 physical numbers: 45,522 B carried of a 27,010,448 B ZIM.
        val ppm = KiwixMetrics.ratioPpm(45_522, 27_010_448)
        assertEquals(1685, ppm) // 0.1685%
        assertEquals("0.1685% (1685 ppm)", KiwixMetrics.formatRatio(ppm))
        // Degenerate inputs never divide by zero.
        assertEquals(0, KiwixMetrics.ratioPpm(100, 0))
        assertEquals(0, KiwixMetrics.ratioPpm(-1, 100))
    }

    @Test
    fun `adapter params JSON-escape paths and carry the strategy`() {
        val p = KiwixParams.adapterParams(
            zimPath = "/sdcard/And roid/data/com.carryon.app/files/\"math\".zim",
            articlePath = "Эйнштейн,_Альберт",
            scrollY = 42,
            strategy = KiwixStrategy.PROGRESSIVE,
        )
        val o = JSONObject(p) // must parse
        assertEquals("/sdcard/And roid/data/com.carryon.app/files/\"math\".zim", o.getString("zim_path"))
        assertEquals("Эйнштейн,_Альберт", o.getString("article_path"))
        assertEquals(42L, o.getLong("scroll_y"))
        assertEquals("D", o.getString("strategy"))
        // Strategy A is a distinct wire value (§6.6 bulk sealing).
        val a = JSONObject(KiwixParams.adapterParams("/x.zim", "index", 0, KiwixStrategy.FULL_UP_FRONT))
        assertEquals("A", a.getString("strategy"))
    }

    @Test
    fun `create_session request names the compiled-in adapter`() {
        val o = JSONObject(KiwixParams.createSessionReq())
        assertEquals(KIWIX_ADAPTER_ID, o.getString("adapter_id"))
        assertEquals("single_writer", o.getString("authority_mode"))
    }

    /** The engine's reader.restore action result, verbatim from the Phase-2
     *  device log (math-mini ZIM) — the exact shape the app parses. */
    @Test
    fun `reader restore output parses the engine shape`() {
        val json = """
        {"oracle":{"agreed":true,"checked":true,"detail":"reading restored at generation 1 (index @ scroll 42)"},
         "output":{"article_hash":"795cdfcd45285b3432ab3668d2d76a77a52a182eb600a257e707a79d6404bede",
                   "article_path":"index","generation":1,"scroll_y":42,"tab_index":0,
                   "zim_size":27010448,"zim_uuid":"e648a210-47dd-5898-33e1-88f16a0ec2c8"},
         "output_hash":"768d99c9cd156db4d2beba43d60d00d3b5b9f461f7d9673db327c275dfdcd896"}
        """.trimIndent()
        val r = RestoredSession.parseActionResult(json)
        assertEquals("e648a210-47dd-5898-33e1-88f16a0ec2c8", r.zimUuid)
        assertEquals(27_010_448L, r.zimSize)
        assertEquals("index", r.articlePath)
        assertEquals(42L, r.scrollY)
        assertEquals(1L, r.generation)
        assertTrue(r.articleHash.length == 64)
    }

    @Test
    fun `formatBytes is honest across the ZIM size range`() {
        assertEquals("27.0 MB", KiwixMetrics.formatBytes(27_010_448))
        assertEquals("1.72 GB", KiwixMetrics.formatBytes(1_720_000_000))
        assertEquals("45.5 KB", KiwixMetrics.formatBytes(45_522))
        assertEquals("512 B", KiwixMetrics.formatBytes(512))
    }
}
