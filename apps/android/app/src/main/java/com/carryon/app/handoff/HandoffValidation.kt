/*
 * HandoffValidation.kt — the L0/L1 file handoff's pure logic (the Android
 * mirror of CarryOnKit/Handoff.swift's validation): §18.6 chunk clamp (the
 * stage-1 device-bug class), manual-IP receiver address validation, pin
 * validation, the §3.8-safe handoff descriptor carried in the engine's
 * Hello/Welcome feature negotiation, the CORE-004 integrity gate, and the
 * ACTION_VIEW mime map (§10.3 PLAT-003).
 *
 * Descriptor law (§3.8): the descriptor is DATA ONLY — a session id, a cut
 * number, one content digest, one file name, one mime type, one evidence
 * class. It never names executable code, a class name, or a shell string, and
 * unknown features are ignored, never interpreted. A destination refuses to
 * guess: a missing/malformed field fails closed.
 *
 * Claim labels (§2): these helpers make no transport claim. The handoff they
 * serve is OBSERVED (LOCAL) on loopback and OBSERVED (PHYSICAL) across two
 * real machines (e.g. the 2026-11-07 Galaxy A04 run); neither is a supported
 * platform (§30 unrun).
 */
package com.carryon.app.handoff

import java.security.MessageDigest

/** Configuration validation for the manual-IP handoff. Honest and strict. */
object HandoffValidation {
    /** §18.6: the engine admits chunk sizes in [4096, 8 MiB] (see carryon-core
     *  MIN/MAX_CHUNK_SIZE). The UI knob is clamped into the admitted range
     *  instead of failing the run — the 2026-11-07 stage-1 device bug (a 256 B
     *  toggle died at OPEN with E_TRANSFER_QUOTA). Chunking is
     *  DESTINATION-controlled (§18.6): the receiver sets it before importing. */
    const val CHUNK_MIN_BYTES = 4096L
    const val CHUNK_MAX_BYTES = 8_388_608L

    fun clampChunkSize(requested: Long): Long =
        requested.coerceIn(CHUNK_MIN_BYTES, CHUNK_MAX_BYTES)

    /** A receiver (source) address for manual mode: `IPv4:port`, strict form
     *  (no leading zeros, port 1..65535). Loopback is valid — it is how the
     *  LOCAL evidence class is produced and labeled honestly. */
    fun isValidReceiverAddr(s: String): Boolean {
        val colon = s.lastIndexOf(':')
        if (colon <= 0 || colon == s.length - 1) return false
        val host = s.substring(0, colon)
        val portText = s.substring(colon + 1)
        if (portText.length > 5 || portText.startsWith("0")) return false
        val port = portText.toLongOrNull() ?: return false
        if (port !in 1..65535) return false
        val parts = host.split('.')
        if (parts.size != 4) return false
        return parts.all { p ->
            p.length in 1..3 && p.all { it.isDigit() } && (p.toLongOrNull() ?: -1) in 0..255
        }
    }

    /** A pairing pin is the 64-hex SHA-256 of a certificate DER (§18.4) — a
     *  fingerprint, never a secret (EVD-006 excludes only private keys). */
    fun isValidPinHex(s: String): Boolean =
        s.length == 64 && s.all { it in '0'..'9' || it in 'a'..'f' }

    /** The evidence class of a run, derived from the address the source dials:
     *  loopback → LOCAL (one device), anything else → PHYSICAL (two machines). */
    fun evidenceClass(dialing: String): String {
        val host = dialing.substringBefore(':')
        return if (host == "127.0.0.1" || host == "localhost") "local" else "physical"
    }

    /** sha256 of bytes, 64 lowercase hex — identical to the engine's
     *  content-addressed digest of `file.document.v1` (adapter-file
     *  hex_sha256) and to the macOS side. */
    fun sha256Hex(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    /** MIME type from a file extension — the ACTION_VIEW intent type. Unknown
     *  extensions stay `application/octet-stream` (the receiver still carries
     *  and verifies the file; only the launch intent depends on the mime). */
    fun mimeForExtension(ext: String): String = when (ext.lowercase()) {
        "png" -> "image/png"
        "jpg", "jpeg" -> "image/jpeg"
        "gif" -> "image/gif"
        "webp" -> "image/webp"
        "heic" -> "image/heic"
        "pdf" -> "application/pdf"
        "txt" -> "text/plain"
        "md" -> "text/markdown"
        "json" -> "application/json"
        "csv" -> "text/csv"
        "mp4" -> "video/mp4"
        "webm" -> "video/webm"
        "mp3" -> "audio/mpeg"
        "html" -> "text/html"
        else -> "application/octet-stream"
    }
}

/**
 * The handoff descriptor the SOURCE sends to the destination inside the
 * engine's Hello/Welcome feature negotiation (relayed verbatim by both FFIs;
 * the engine attaches no semantics). Data only (§3.8).
 */
data class HandoffDescriptor(
    val session: String,       // source session id (import_cut's remote_session)
    val cut: Long,             // sealed cut number (import_cut's remote_cut)
    val contentHash: String,   // declared sha256 of file.document.v1 (64 hex)
    val fileName: String,      // display name for the reconstructed file
    val mimeType: String,      // ACTION_VIEW intent type (§10.3 PLAT-003)
    val evidenceClass: String, // "local" | "physical" — the source's honest label
) {
    /** The Hello features this source sends (key=value after a marker). */
    fun features(): List<String> = listOf(
        MARKER,
        "session=$session",
        "cut=$cut",
        "hash=$contentHash",
        "name=$fileName",
        "mime=$mimeType",
        "evid=$evidenceClass",
    )

    companion object {
        const val MARKER = "org.carryon.handoff.v1"

        /** Parse out of the peer's Hello features. Fail closed on a missing
         *  marker, duplicate keys, or a malformed hash: a destination never
         *  guesses its import parameters (§3.8 — refuse, do not infer). */
        fun parse(features: List<String>): HandoffDescriptor? {
            if (MARKER !in features) return null
            var session: String? = null
            var cut: Long? = null
            var hash: String? = null
            var name: String? = null
            var mime: String? = null
            var evid: String? = null
            for (f in features) {
                val eq = f.indexOf('=') + 1
                if (eq == 0) continue
                val key = f.substring(0, eq - 1)
                val value = f.substring(eq)
                when (key) {
                    "session" -> if (session == null) session = value
                    "cut" -> if (cut == null) cut = value.toLongOrNull()
                    "hash" -> if (hash == null) hash = value
                    "name" -> if (name == null) name = value
                    "mime" -> if (mime == null) mime = value
                    "evid" -> if (evid == null) evid = value
                    else -> Unit // unknown keys are ignored, never interpreted
                }
            }
            val s = session ?: return null
            val c = cut ?: return null
            val h = hash ?: return null
            val n = name ?: return null
            val m = mime ?: return null
            val e = evid ?: return null
            if (!HandoffValidation.isValidPinHex(h)) return null
            if (n.isEmpty() || m.isEmpty()) return null
            if (e != "local" && e != "physical") return null
            return HandoffDescriptor(s, c, h, n, m, e)
        }
    }
}

/**
 * The destination-side CORE-004 discipline as a pure, testable decision:
 * bytes reconstructed from the store are exposed to the user (or to an
 * ACTION_VIEW intent) ONLY when their measured sha256 equals the declared
 * content hash. The engine guarantees the store itself (stage → verify →
 * atomic publish, CORE-004); this gate guards the app's reconstruction path
 * and refuses closed.
 */
object IntegrityGate {
    fun verify(bytes: ByteArray, declaredHash: String): Boolean =
        HandoffValidation.sha256Hex(bytes) == declaredHash.lowercase()
}
