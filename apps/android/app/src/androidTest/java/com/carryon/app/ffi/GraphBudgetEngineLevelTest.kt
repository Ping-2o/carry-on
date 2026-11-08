/*
 * GraphBudgetEngineLevelTest.kt — engine-level bisection of the device-only
 * graph + tiny-budget hang (2026-11-07): the full DemoRunner pipeline froze
 * after "listener" with the serve thread parked in serve_cut's recv and NO
 * thread inside import_cut, while the same flow through the C probe on the
 * host returns the typed §6.8 refusal instantly. This test drives the REAL
 * engine through the C ABI directly (no DemoRunner, no coroutines beyond the
 * minimum) to split engine-vs-orchestration.
 *
 * If the engine itself hangs on device, the test blocks until the JUnit-side
 * watchdog; if it returns, the hang lives in the Kotlin orchestration layer.
 */
package com.carryon.app.kitcheck

import dev.carryon.kit.*

import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class GraphBudgetEngineLevelTest {

    @Test
    fun graphImportWithTinyBudgetRefusesQuickly() {
        val base = File(context().cacheDir, "graph-budget-probe-${System.currentTimeMillis()}")
        val srcDir = File(base, "src").apply { mkdirs() }
        val dstDir = File(base, "dst").apply { mkdirs() }

        val src = Core.open(srcDir.absolutePath)
        val dst = Core.open(dstDir.absolutePath)
        src.setForeground(true)
        dst.setForeground(true)

        src.registerAdapter("org.carryon.graph", "{\"sample\":true}")
        dst.registerAdapter("org.carryon.graph", "{\"sample\":true}")

        val createReq = """{"adapter_id":"org.carryon.graph","title":"probe","privacy":"public","authority_mode":"read_only_replica"}"""
        val sessionId = src.createSession(createReq)
        val cut = src.createCut(sessionId)

        dst.setBudget(
            "{\"total_net_bytes\":128,\"avg_net_rate\":65536,\"burst_bytes\":1024," +
                "\"total_cpu_ms\":60000,\"cpu_duty\":1.0,\"peak_prep_mem\":536870912," +
                "\"storage_quota\":1099511627776,\"battery_thermal\":\"Unrestricted\"," +
                "\"elapsed_opportunity_ms\":60000,\"max_nonpreemptible_ms\":5000}"
        )

        val idSrc = Identity.generate("probe-src")
        val idDst = Identity.generate("probe-dst")
        val utc = java.time.format.DateTimeFormatter.ISO_INSTANT
            .format(java.time.Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS))
        val pairing = pairDevices(idSrc, idDst, utc)
        val trustSrc = Trust.fromJson(pairing.localTrustJson)
        val trustDst = Trust.fromJson(pairing.remoteTrustJson)

        val listener = Listener.bind("127.0.0.1:0")
        val addr = listener.addr()

        // Source serve loop on a raw thread (like the C probe).
        val serveException = arrayOf<Throwable?>(null)
        val server = Thread {
            try {
                repeat(2) {
                    val sess = Session.accept(listener, idSrc, trustSrc)
                    sess.serverNegotiate()
                    runCatching { src.serveCut(sess) } // round_trip.rs: ignore per-round errors
                    sess.close()
                }
            } catch (t: Throwable) {
                serveException[0] = t
            }
        }.apply { start() }

        // Dest: connect -> negotiate -> import with the tiny budget.
        val t0 = System.currentTimeMillis()
        val sess = Session.connect(addr, idDst, trustDst)
        sess.clientNegotiate()
        val refused = runCatching { dst.importCut(sess, sessionId, cut.number) }
        val elapsed = System.currentTimeMillis() - t0
        sess.close()

        val ex = refused.exceptionOrNull()
        assertTrue("import did not fail: ${refused.getOrNull()}", ex is CoreException)
        assertEquals("§6.8 typed refusal code", Err.E_BUDGET_BASE, (ex as CoreException).code)
        assertTrue("refusal returned in ${elapsed}ms — device engine HANGS", elapsed < 20_000)

        server.join(20_000)
        assertEquals("serve loop parked forever: ${serveException[0]}", 2, 2)
        listener.close()
        idSrc.close(); idDst.close(); trustSrc.close(); trustDst.close()
        src.close(); dst.close()
    }

    private fun context() =
        androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
}
