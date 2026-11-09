/*
 * KiwixAssetsTest.kt — JVM tests for the pure half of ZIM asset serving
 * (#25): URL <-> ZIM path mapping and the fail-closed parse of reader.asset
 * output (digest + size re-check). The engine half (libzim lookup) runs in
 * the Rust crate's tests and on a device.
 *
 * Labels (§2): pure-logic tests; no device claim.
 */
package com.carryon.app.kiwix

import org.json.JSONObject
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import java.security.MessageDigest
import java.util.Base64

class KiwixAssetsTest {

    private val png = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3)

    private fun sha256(b: ByteArray) =
        MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it.toInt() and 0xff) }

    private fun result(
        bytes: ByteArray = png,
        sha: String = sha256(png),
        size: Long = png.size.toLong(),
        mime: String = "image/png",
        b64: String = Base64.getEncoder().encodeToString(bytes),
    ): String = JSONObject()
        .put(
            "output",
            JSONObject()
                .put("path", "I/pic.png")
                .put("mimetype", mime)
                .put("size", size)
                .put("sha256", sha)
                .put("data_b64", b64),
        )
        .put("oracle", JSONObject())
        .toString()

    @Test
    fun mapsOurUrlsToZimPaths() {
        assertEquals("I/m/Pic.png", KiwixAssets.zimPathFor("https://kiwix.local/I/m/Pic.png"))
        assertEquals("I/a b.png", KiwixAssets.zimPathFor("https://kiwix.local/I/a%20b.png?x=1#f"))
        assertEquals("_assets_/style.css", KiwixAssets.zimPathFor("https://KIWIX.local/A/../_assets_/style.css"))
    }

    @Test
    fun blocksEverythingElse() {
        assertNull(KiwixAssets.zimPathFor("https://example.com/I/x.png"))
        assertNull(KiwixAssets.zimPathFor("http://kiwix.local/I/x.png"))
        assertNull(KiwixAssets.zimPathFor("https://kiwix.local/"))
        assertNull(KiwixAssets.zimPathFor("https://kiwix.local/A/../../etc/passwd"))
        assertNull(KiwixAssets.zimPathFor("data:text/html,hi"))
        assertNull(KiwixAssets.zimPathFor("not a url"))
    }

    @Test
    fun baseUrlRoundTripsNonAsciiArticlePaths() {
        assertEquals("https://kiwix.local/A/Foo", KiwixAssets.baseUrlFor("A/Foo"))
        for (p in listOf("Физика", "Эйнштейн,_Альберт", "index")) {
            assertEquals(p, KiwixAssets.zimPathFor(KiwixAssets.baseUrlFor(p)))
        }
    }

    @Test
    fun parsesAVerifiedAsset() {
        val a = KiwixAsset.parseActionResult(result())
        assertNotNull(a)
        assertArrayEquals(png, a!!.bytes)
        assertEquals("image/png", a.mimeType)
        assertEquals("I/pic.png", a.path)
    }

    @Test
    fun stripsMimeParametersAndDefaultsBadMimes() {
        assertEquals("text/css", KiwixAsset.parseActionResult(result(mime = "text/css; charset=utf-8"))!!.mimeType)
        assertEquals("application/octet-stream", KiwixAsset.parseActionResult(result(mime = ""))!!.mimeType)
    }

    @Test
    fun failsClosedOnDigestSizeOrEncodingMismatch() {
        assertNull(KiwixAsset.parseActionResult(result(sha = "00".repeat(32))))
        assertNull(KiwixAsset.parseActionResult(result(size = png.size + 1L)))
        assertNull(KiwixAsset.parseActionResult(result(b64 = "!!!not-base64!!!")))
        assertNull(KiwixAsset.parseActionResult("{\"oracle\":{}}"))
        assertNull(KiwixAsset.parseActionResult("not json"))
    }
}
