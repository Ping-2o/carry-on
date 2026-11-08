/*
 * KiwixSessionInstrumentedTest.kt — the REAL engine + REAL libzim on a real
 * device (stage 10, mobile half; instrumented: a JVM test cannot load the
 * compiled-in kiwix adapter or read a ZIM).
 *
 * Precondition (device setup — the test fails fast with a clear message):
 *   adb push wikipedia_ru_mathematics_mini_2026-10.zim \
 *     /sdcard/Android/data/com.carryon.app/files/
 *
 * Proves, on-device: open ZIM → session+cut → reader.restore (article hash
 * matches the bytes read back from the content-addressed store) →
 * reader.navigate to a REAL entry (generation bump) → a missing entry is
 * refused fail-closed → a stale cut after the move is refused.
 *
 * Requires an engine built with --features kiwix (Scripts/fetch_engine_lib.sh
 * with CARRYON_KIWIX=1). Without it register_adapter fails and this test
 * reports the honest adapter-not-compiled-in error.
 */
package com.carryon.app.kiwix

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.carryon.app.ffi.Core
import com.carryon.app.ffi.CoreException
import com.carryon.app.ffi.Err
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class KiwixSessionInstrumentedTest {

    private val context: Context = ApplicationProvider.getApplicationContext()
    private val zim = File(
        context.getExternalFilesDir(null),
        "wikipedia_ru_mathematics_mini_2026-10.zim",
    )

    @Test
    fun openRestoreNavigateThroughRealLibzim() {
        assumeTrue(
            "ZIM not pushed: adb push wikipedia_ru_mathematics_mini_2026-10.zim /sdcard/Android/data/com.carryon.app/files/",
            zim.isFile,
        )
        val work = File(context.cacheDir, "kiwix-it-${System.currentTimeMillis()}").apply { mkdirs() }
        val core = Core.open(work.absolutePath)
        try {
            core.setForeground(true)
            // Every failure here is a REAL one (adapter not compiled in, bad
            // ZIM, wrong path) — no silent pass.
            core.registerAdapter(
                KIWIX_ADAPTER_ID,
                KiwixParams.adapterParams(zim.absolutePath, "index", 42, KiwixStrategy.PROGRESSIVE),
            )
            val sid = core.createSession(KiwixParams.createSessionReq())
            val cut = core.createCut(sid)
            val cutRef = """{"session":"$sid","number":${cut.number}}"""

            val restored = RestoredSession.parseActionResult(
                core.executeActionAs(cutRef, """{"class":"reader.restore","params":{}}""", KIWIX_ADAPTER_ID),
            )
            assertEquals("index", restored.articlePath)
            assertEquals(42L, restored.scrollY)
            assertTrue("zim size must be the real 27 MB archive", restored.zimSize > 20_000_000)
            assertEquals("math-mini ZIM uuid (Phase-2 capture)", "e648a210-47dd-5898-33e1-88f16a0ec2c8", restored.zimUuid)

            // The article bytes really come back from the store, digest-checked
            // by the core (CORE-004): non-empty HTML and its sha256 matches the
            // declared article_hash.
            val html = core.readObjectBytes(restored.articleHash)
            assertTrue("article bytes empty", html.isNotEmpty())
            val ours = java.security.MessageDigest.getInstance("SHA-256").digest(html)
                .joinToString("") { "%02x".format(it) }
            assertEquals(restored.articleHash, ours)
            assertTrue("not HTML", String(html, Charsets.UTF_8).contains("<"))

            // L4 on-device: navigate to a REAL second entry (the Einstein
            // article used by the Phase-3 physical run) → generation 2.
            val nav = core.executeActionAs(
                cutRef,
                """{"class":"reader.navigate","params":{"path":"Эйнштейн,_Альберт","scroll_y":321}}""",
                KIWIX_ADAPTER_ID,
            )
            assertTrue(nav.contains("\"generation\":2"))

            // Fail closed on a missing entry: the real ZIM is consulted.
            val missing = runCatching {
                core.executeActionAs(
                    cutRef,
                    """{"class":"reader.navigate","params":{"path":"no-such-article-xyz","scroll_y":0}}""",
                    KIWIX_ADAPTER_ID,
                )
            }
            assertTrue("missing ZIM entry must fail closed", missing.isFailure)

            // The old cut is stale after the move: restore-at-generation-1 is
            // refused (the carried cut sealed the OLD generation).
            val stale = runCatching {
                core.executeActionAs(cutRef, """{"class":"reader.restore","params":{}}""", KIWIX_ADAPTER_ID)
            }
            assertTrue("stale-cut action must be refused", stale.isFailure)
            assertNotEquals(0, (stale.exceptionOrNull() as? CoreException)?.code ?: Err.UNKNOWN)
        } finally {
            runCatching { core.close() }
        }
    }
}
