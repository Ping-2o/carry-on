/*
 * ContractParityTest.kt — the SDK contract scenarios on the Kotlin side. The
 * SAME scenario table runs through the Swift SDK
 * (sdk/swift/Tests/CarryOnKitTests/ContractParityTests.swift) — these are the
 * parity contract tests:
 *
 *   carry-file/loopback-positive   file → cut → TLS → descriptor → import →
 *                                  CORE-004 gate → document.open, byte-for-byte
 *   carry-file/tampered-refused    declared hash tampered → REFUSED (fail closed)
 *   session/transfer-evidence      two cores, sample editor session, import,
 *                                  evidence bundle verifies ok
 *   abi/version-gate               the runtime gate + SDK version follows the ABI
 *
 * Runs on the HOST JVM against the desktop JNI shim
 * (apps/android/Scripts/build_desktop_shim.sh); without it the engine-level
 * tests skip and only the pure-logic suite runs. The device matrix
 * (instrumented) remains the platform-level evidence — labels follow §2:
 * host JVM = contract proof, not an Android platform claim.
 */
package dev.carryon.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicInteger

class ContractParityTest {

    private val libDir: String? = System.getProperty("carryon.desktop.libdir")

    @Before
    fun requireDesktopShim() {
        assumeTrue("desktop shim not configured (set carryonDesktopLibDir)", libDir != null)
    }

    private fun root(tag: String): File =
        File(System.getProperty("java.io.tmpdir"), "carryon-kit-contract-$tag-${System.nanoTime()}")
            .apply { mkdirs() }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    /**
     * The handoff descriptor format the apps use (data only, §3.8): a marker
     * plus key=value strings over the engine's Hello/Welcome features. The SDK
     * test sends the same shape through the same channel.
     */
    private fun features(session: String, cut: Long, hash: String, name: String, mime: String) =
        listOf(
            "org.carryon.handoff.v1", "session=$session", "cut=$cut", "hash=$hash",
            "name=$name", "mime=$mime", "evid=local",
        )

    /** Parse the peer's features JSON array (the engine relays them verbatim). */
    private fun parseFeatures(json: String): Map<String, String> {
        val arr = org.json.JSONArray(json)
        val out = HashMap<String, String>()
        for (i in 0 until arr.length()) {
            val f = arr.getString(i)
            val eq = f.indexOf('=')
            if (eq > 0) out[f.substring(0, eq)] = f.substring(eq + 1)
        }
        return out
    }

    // ---- carry-file/loopback-positive ---------------------------------------

    @Test
    fun carryFileLoopbackPositive() {
        CarryOnKit.requireCompatibleEngine()
        val out = runCarryFile(tamper = false)
        assertTrue("destination must verify measured sha256 == declared", out.verified)
        assertFalse(out.refused)
        assertTrue("engine document.open activation record must run", out.opened)
    }

    // ---- carry-file/tampered-refused -----------------------------------------

    @Test
    fun carryFileTamperedRefused() {
        CarryOnKit.requireCompatibleEngine()
        val out = runCarryFile(tamper = true)
        assertFalse("no success may be reported on a mismatched object", out.verified)
        assertTrue("tampered declared hash must be REFUSED (CORE-004)", out.refused)
    }

    // ---- session/transfer-evidence -------------------------------------------

    @Test
    fun sessionTransferEvidenceVerifies() {
        CarryOnKit.requireCompatibleEngine()
        val dir = root("evidence")
        val src = Core.open(File(dir, "src").absolutePath)
        val dst = Core.open(File(dir, "dst").absolutePath)
        try {
            src.registerAdapter("org.carryon.editor", "{\"session_v1\":true}")
            dst.registerAdapter("org.carryon.editor", "{\"session_v1\":true}")
            val session = src.createSession(
                """{"adapter_id":"org.carryon.editor","title":"contract","privacy":"personal","authority_mode":"read_only_replica"}"""
            )
            val cut = src.createCut(session)
            val srcId = Identity.generate("contract-src")
            val dstId = Identity.generate("contract-dst")
            val pairing = pairDevices(srcId, dstId, java.time.format.DateTimeFormatter.ISO_INSTANT
                .format(java.time.Instant.now()))
            Trust.fromJson(pairing.localTrustJson).use { srcTrust ->
                Trust.fromJson(pairing.remoteTrustJson).use { dstTrust ->
                    Listener.bind("127.0.0.1:0").use { listener ->
                        val addr = listener.addr()
                        val serveResult = AtomicInteger(-999)
                        val server = Thread {
                            try {
                                Session.accept(listener, srcId, srcTrust).use { wire ->
                                    wire.serverNegotiate("[]")
                                    src.serveCut(wire)
                                    serveResult.set(0)
                                }
                            } catch (t: Throwable) {
                                serveResult.set(-1)
                            }
                        }.apply { start() }
                        Session.connect(addr, dstId, dstTrust).use { wire ->
                            wire.clientNegotiate("[]")
                            val outcome = dst.importCut(wire, session, cut.number)
                            assertTrue(outcome is ImportOutcome.Completed)
                        }
                        server.join(30_000)
                        assertEquals("serve leg failed", 0, serveResult.get())

                        val mirror = mirrorSessionId(session)
                        val bundle = dst.exportEvidenceWith(mirror, """{"level":"L0-L1-file-handoff","contract":"parity"}""")
                        val bundleFile = File(File(dir, "dst"), "evidence.json").apply { writeText(bundle) }
                        val report = dst.verifyEvidence(bundleFile.absolutePath)
                        assertTrue("evidence must verify: $report",
                            org.json.JSONObject(report).optBoolean("ok", false))
                    }
                }
            }
        } finally {
            runCatching { src.close() }
            runCatching { dst.close() }
        }
    }

    // ---- abi/version-gate -----------------------------------------------------

    @Test
    fun abiVersionGateIsEnforced() {
        CarryOnKit.requireCompatibleEngine()
        val engine = abiVersion()
        assertEquals(CarryOnKit.ABI_MAJOR, engine.major)
        assertTrue(engine.minor >= CarryOnKit.ABI_MINOR)
    }

    // ---- shared runner (mirrors the Swift ContractParityTests) ----------------

    private class Outcome {
        @Volatile var verified = false
        @Volatile var refused = false
        @Volatile var opened = false
        @Volatile var error: String? = null
    }

    private fun runCarryFile(tamper: Boolean): Outcome {
        val dir = root("carryfile")
        val carried = File(dir, "carryon-doc.png").apply {
            writeBytes(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A) +
                ByteArray(256) { (it % 251).toByte() })
        }
        val trueHash = sha256(carried.readBytes())
        // The negative test declares the digest of TAMPERED bytes.
        val declared = if (tamper) sha256(carried.readBytes() + 0x00) else trueHash

        val out = Outcome()
        val dstCore = Core.open(File(dir, "dst").absolutePath)
        val placeholder = File(dir, "placeholder.txt").apply { writeText("placeholder") }
        dstCore.registerAdapter("org.carryon.file", "{\"path\":\"${placeholder.absolutePath}\"}")
        val dstId = Identity.generate("contract-dest")
        val srcId = Identity.generate("contract-src")

        Listener.bind("127.0.0.1:0").use { listener ->
            val addr = listener.addr()
            val dstTrust = Trust.empty().apply {
                pair("source", srcId.pinHex(), java.time.format.DateTimeFormatter.ISO_INSTANT
                    .format(java.time.Instant.now()))
            }
            val receiver = Thread {
                try {
                    dstTrust.use { trust ->
                        Session.accept(listener, dstId, trust).use { wire ->
                            val theirs = parseFeatures(wire.serverNegotiate("[]"))
                            val hash = theirs["hash"] ?: error("descriptor malformed")
                            val session = theirs["session"] ?: error("descriptor malformed")
                            val cut = theirs["cut"]?.toLongOrNull() ?: error("descriptor malformed")
                            dstCore.importCut(wire, session, cut)
                            // CORE-004 app-level gate: declared == measured, or refuse.
                            if (dstCore.hasObjectHex(hash)) {
                                val bytes = dstCore.readObjectBytes(hash)
                                if (sha256(bytes) == hash.lowercase()) {
                                    out.verified = true
                                    val mirror = mirrorSessionId(session)
                                    dstCore.executeActionAs(
                                        """{"session":"$mirror","number":$cut}""",
                                        """{"class":"document.open","params":{"uri":"content://contract/${theirs["name"]}"}}""",
                                        "org.carryon.file",
                                    )
                                    out.opened = true
                                } else {
                                    out.refused = true
                                }
                            } else {
                                out.refused = true
                            }
                        }
                    }
                } catch (t: Throwable) {
                    out.error = t.message ?: "$t"
                }
            }.apply { start() }

            // Source side: the same steps the apps perform.
            val srcCore = Core.open(File(dir, "src").absolutePath)
            try {
                srcCore.registerAdapter("org.carryon.file", "{\"path\":\"${carried.absolutePath}\"}")
                val session = srcCore.createSession(
                    """{"adapter_id":"org.carryon.file","title":"contract file","privacy":"personal","authority_mode":"read_only_replica"}"""
                )
                val cut = srcCore.createCut(session)
                val srcTrust = Trust.empty().apply {
                    pair("receiver", dstId.pinHex(), java.time.format.DateTimeFormatter.ISO_INSTANT
                        .format(java.time.Instant.now()))
                }
                srcTrust.use { trust ->
                    Session.connect(addr, srcId, trust).use { wire ->
                        wire.clientNegotiate(org.json.JSONArray(
                            features(session, cut.number, declared, "carryon-doc.png", "image/png")
                        ).toString())
                        srcCore.serveCut(wire)
                    }
                }
            } finally {
                runCatching { srcCore.close() }
            }
            receiver.join(30_000)
        }
        runCatching { dstCore.close() }
        out.error?.let { throw AssertionError("destination failed: $it") }
        return out
    }
}
