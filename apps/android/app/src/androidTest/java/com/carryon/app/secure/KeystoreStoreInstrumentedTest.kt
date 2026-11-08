/*
 * KeystoreStoreInstrumentedTest.kt — PLAT-002 on REAL hardware (mandatory:
 * a JVM unit test cannot prove AndroidKeyStore). Proves on the device:
 *
 *   1. the master key lives in AndroidKeyStore (not app storage);
 *   2. the identity's private key DER NEVER appears in plaintext on disk —
 *      only GCM-sealed ciphertext (EVD-006/§22.3);
 *   3. the pin is STABLE across "launches" (loadOrCreate → restore);
 *   4. tamper-negative: a flipped ciphertext bit fails the GCM tag and the
 *      store regenerates a FRESH identity (fail closed, never half-trusted);
 *   5. the trust JSON round-trips sealed.
 *
 * Claim label (§2 law): OBSERVED on-device — this proves the secure-storage
 * layer, not any platform "supported" claim (§30 unrun).
 */
package com.carryon.app.secure

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.carryon.app.ffi.Identity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.security.KeyStore

@RunWith(AndroidJUnit4::class)
class KeystoreStoreInstrumentedTest {

    private fun context(): Context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun masterKeyLivesInAndroidKeyStore() {
        val key = KeystoreIdentityStore.masterKey()
        assertNotNull(key)
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        assertTrue("master key must be IN AndroidKeyStore", ks.containsAlias("carryon_master"))
        // Non-exportable by construction: the entry is a SecretKeyEntry the
        // OS refuses to materialize outside Keystore (no getEncoded contract).
        assertTrue(ks.getEntry("carryon_master", null) is KeyStore.SecretKeyEntry)
    }

    @Test
    fun identityStableAcrossLaunches_andKeyDerNeverOnDiskInPlaintext() {
        val ctx = context()
        // "Launch" 1: create.
        val id1 = KeystoreIdentityStore.loadOrCreateIdentity(ctx, "carryon-a04")
        id1.use {
            val pin1 = it.pinHex()
            // The SECRET (key DER) — read here ONLY to prove the disk copy is
            // sealed; never logged, never bundled (EVD-006).
            val keyDer = it.keyDer()
            val certDer = it.certDer()

            // "Launch" 2: restore — stable pin.
            val id2 = KeystoreIdentityStore.loadOrCreateIdentity(ctx, "carryon-a04")
            id2.use {
                assertEquals("pin must be stable across launches (§18.4)", pin1, it.pinHex())

                // PLAT-002's core negative: NO plaintext key DER anywhere in
                // app storage — scan every file for the raw bytes.
                val files = File(ctx.filesDir, "secure").listFiles() ?: emptyArray()
                assertTrue("sealed records must exist", files.isNotEmpty())
                files.forEach { f ->
                    val disk = f.readBytes()
                    assertFalse(
                        "PLAINTEXT KEY DER LEAKED in ${f.name} (EVD-006 violation)",
                        containsSubarray(disk, keyDer),
                    )
                    assertFalse(
                        "plaintext cert DER unwrapped in ${f.name}?",
                        containsSubarray(disk, certDer),
                    )
                }
            }
        }
    }

    @Test
    fun tamperedCiphertextFailsClosed_freshIdentity() {
        val ctx = context()
        val id1 = KeystoreIdentityStore.loadOrCreateIdentity(ctx, "carryon-a04")
        val pin1 = id1.use { it.pinHex() }

        // Flip one bit of the sealed KEY record → GCM tag must fail → the
        // store treats it as absent and generates a FRESH identity.
        val keyFile = File(File(ctx.filesDir, "secure"), "device_key.bin")
        assertTrue(keyFile.exists())
        val bytes = keyFile.readBytes()
        bytes[bytes.size - 1] = (bytes[bytes.size - 1].toInt() xor 0x01).toByte()
        keyFile.writeBytes(bytes)

        val id2 = KeystoreIdentityStore.loadOrCreateIdentity(ctx, "carryon-a04")
        id2.use {
            assertNotEquals(
                "a tampered sealed identity must NOT restore (fail closed)",
                pin1, it.pinHex(),
            )
        }
    }

    @Test
    fun trustJsonRoundTripsSealed() {
        val ctx = context()
        val json = """{"peers":{"${"a".repeat(64)}":{"device_name":"mac","pin_hex":"${"a".repeat(64)}","trust_generation":0,"revoked":false,"paired_utc":"t"}}}"""
        KeystoreIdentityStore.saveTrust(ctx, json)
        assertEquals(json, KeystoreIdentityStore.loadTrust(ctx))
    }

    private fun containsSubarray(haystack: ByteArray, needle: ByteArray): Boolean {
        if (needle.isEmpty() || haystack.size < needle.size) return false
        outer@ for (i in 0..haystack.size - needle.size) {
            for (j in needle.indices) {
                if (haystack[i + j] != needle[j]) continue@outer
            }
            return true
        }
        return false
    }
}
