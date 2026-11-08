/*
 * KeystoreIdentityStore.kt — PLAT-002 secure storage for the Carry-On
 * Android app: the device identity's PRIVATE KEY DER and the TrustStore JSON
 * live ONLY wrapped under an AndroidKeyStore key. The plaintext key DER never
 * touches disk (no file, no SharedPreferences, no log — EVD-006/§22.3).
 *
 * How (the standard AndroidKeyStore pattern): a non-exportable AES-256-GCM
 * key is generated INSIDE AndroidKeyStore (alias "carryon_master"). The key
 * DER + trust JSON are sealed with it (AES/GCM, per-record IV + auth tag) and
 * only the CIPHERTEXT is written to app-private storage. Keystore hardware
 * (StrongBox where available) holds the wrapping key; AndroidKeyStore keys
 * cannot be exported, so the ciphertext alone is inert off-device.
 *
 * Fail-closed: a tampered ciphertext (flipped bit) fails the GCM auth tag and
 * is discarded — the store regenerates a FRESH identity rather than trusting
 * damaged credentials (§18.4: a bad restore fails, never half-trusts).
 *
 * Thread/lifecycle: all engine calls run on Dispatchers.IO (DemoRunner
 * rules); the store is loaded LAZILY at first use so app start never blocks
 * on Keystore (stage-5 optimization requirement).
 */
package com.carryon.app.secure

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import dev.carryon.kit.Identity
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** One sealed record: ciphertext + IV, both non-secret without the Keystore key. */
private data class Sealed(val iv: ByteArray, val ct: ByteArray)

private const val GCM_TAG_BITS = 128

object KeystoreIdentityStore {

    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val MASTER_ALIAS = "carryon_master"
    private const val GCM_TAG_BITS = 128

    /** The slots under which plaintext was NEVER persisted: key + cert + trust. */
    private const val SLOT_KEY = "device_key"
    private const val SLOT_CERT = "device_cert"
    private const val SLOT_TRUST = "device_trust"

    // ---- AndroidKeyStore master key --------------------------------------

    /** Get-or-create the non-exportable AES-256 master key (device test target). */
    fun masterKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getKey(MASTER_ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(
                MASTER_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build()
        )
        return gen.generateKey()
    }

    // ---- seal / unseal ------------------------------------------------------

    private fun seal(plain: ByteArray): Sealed {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, masterKey())
        val ct = cipher.doFinal(plain)
        return Sealed(cipher.iv, ct)
    }

    /** Fail-closed unseal: a tampered/foreign ciphertext throws (GCM tag). */
    private fun unseal(record: Sealed): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, masterKey(), GCMParameterSpec(GCM_TAG_BITS, record.iv))
        return cipher.doFinal(record.ct)
    }

    // ---- persistence (ciphertext only, app-private file) --------------------

    private fun recordsDir(context: Context): File =
        File(context.filesDir, "secure").apply { mkdirs() }

    private fun writeSealed(context: Context, slot: String, plain: ByteArray) {
        val s = seal(plain)
        // Format: <iv-len>:<iv><ct> — parseable, tamper-evident via the tag.
        File(recordsDir(context), "$slot.bin").writeBytes(
            s.iv.size.toString().toByteArray() + ":".toByteArray() + s.iv + s.ct
        )
    }

    private fun readSealed(context: Context, slot: String): ByteArray? {
        val f = File(recordsDir(context), "$slot.bin")
        if (!f.exists()) return null
        val bytes = f.readBytes()
        val sep = bytes.indexOf(':'.code.toByte())
        if (sep <= 0) return null
        val ivLen = String(bytes, 0, sep).toIntOrNull() ?: return null
        val iv = bytes.copyOfRange(sep + 1, sep + 1 + ivLen)
        val ct = bytes.copyOfRange(sep + 1 + ivLen, bytes.size)
        return try {
            unseal(Sealed(iv, ct))
        } catch (e: Exception) {
            // GCM tag mismatch (tamper/foreign ciphertext): fail closed —
            // treat as absent, never as damaged-but-trusted credentials.
            null
        }
    }

    // ---- the device identity (PLAT-002 core) ---------------------------------

    /**
     * The durable device identity: restore from Keystore-wrapped DERs (stable
     * pin across launches, §18.4) or generate + persist a fresh one. The key
     * DER exists in memory only inside the engine after this returns.
     */
    fun loadOrCreateIdentity(context: Context, name: String): Identity {
        val cert = readSealed(context, SLOT_CERT)
        val key = readSealed(context, SLOT_KEY)
        if (cert != null && key != null) {
            try {
                return Identity.fromDer(name, cert, key)
            } catch (e: Exception) {
                // A cert/key that no longer parses fails closed: fresh identity
                // (the old pin simply disappears from this device's store).
            }
        }
        val id = Identity.generate(name)
        writeSealed(context, SLOT_CERT, id.certDer())
        writeSealed(context, SLOT_KEY, id.keyDer()) // SECRET — sealed immediately
        return id
    }

    // ---- the trust store (pins only, but sealed too) ---------------------------

    /** Persist the TrustStore JSON (public pins; sealed per the stage-5 brief). */
    fun saveTrust(context: Context, json: String) {
        writeSealed(context, SLOT_TRUST, json.toByteArray(Charsets.UTF_8))
    }

    /** The persisted TrustStore JSON, or null when nothing is paired yet. */
    fun loadTrust(context: Context): String? =
        readSealed(context, SLOT_TRUST)?.toString(Charsets.UTF_8)
}
