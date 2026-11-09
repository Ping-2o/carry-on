/*
 * KiwixTypes.kt — model + pure helpers of the Kiwix companion reader
 * (stage 10, mobile half): a reading session over a gigantic ZIM is an L3
 * session whose *mutable* state is a few KB (article path + scroll), while
 * the library itself is a local, immutable file. The flagship metric shipped
 * here is `bytes_before_first_action` vs the ZIM size (D/A).
 *
 * Labels are law (§2): a reading run on one device is OBSERVED (LOCAL); a
 * run across two real machines is PHYSICAL. A platform is never "supported"
 * until §30 passes on hardware.
 */
package com.carryon.app.kiwix

import org.json.JSONObject

/** Adapter id of the compiled-in Kiwix adapter (data, never behavior — §3.8). */
const val KIWIX_ADAPTER_ID = "org.carryon.kiwix"

/** The exact §6.6 story in one string: strategy D = progressive (the ZIM bulk
 *  is an unsealed Cache object and never enters the cut); strategy A = the
 *  bulk sealed up front. */
enum class KiwixStrategy(val code: String, val label: String) {
    PROGRESSIVE("D", "D — progressive (ZIM stays local; the cut is KBs)"),
    FULL_UP_FRONT("A", "A — bulk sealed up front (the ZIM would travel)"),
}

/** Adapter params for `org.carryon.kiwix` (factory.rs arm) — DATA naming a
 *  compiled-in adapter. Paths are user filesystem paths; JSON-escaped here. */
object KiwixParams {
    fun jsonEscape(s: String): String = buildString(s.length + 8) {
        for (c in s) {
            when (c) {
                '\\' -> append("\\\\")
                '"' -> append("\\\"")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                else -> append(c)
            }
        }
    }

    fun adapterParams(
        zimPath: String,
        articlePath: String,
        scrollY: Long,
        strategy: KiwixStrategy,
        sessionName: String = "kiwix-companion",
    ): String = buildString {
        append("{\"zim_path\":\"")
        append(jsonEscape(zimPath))
        append("\",\"article_path\":\"")
        append(jsonEscape(articlePath))
        append("\",\"scroll_y\":")
        append(scrollY)
        append(",\"strategy\":\"")
        append(strategy.code)
        append("\",\"session\":\"")
        append(jsonEscape(sessionName))
        append("\"}")
    }

    fun createSessionReq(title: String = "kiwix reading session"): String =
        "{\"adapter_id\":\"$KIWIX_ADAPTER_ID\",\"title\":\"${jsonEscape(title)}\"," +
            "\"privacy\":\"personal\",\"authority_mode\":\"single_writer\"}"
}

/** Parsed `reader.restore` output (adapter execute_action) — the restored
 *  logical session a destination proves it holds. */
data class RestoredSession(
    val zimUuid: String,
    val zimSize: Long,
    val articlePath: String,
    val scrollY: Long,
    val tabIndex: Long,
    val articleHash: String,
    val generation: Long,
) {
    companion object {
        /** From the action result JSON `{"output":{…},"oracle":{…}}`. */
        fun parseActionResult(json: String): RestoredSession {
            val o = JSONObject(JSONObject(json).getJSONObject("output").toString())
            return RestoredSession(
                zimUuid = o.optString("zim_uuid"),
                zimSize = o.optLong("zim_size", 0),
                articlePath = o.getString("article_path"),
                scrollY = o.optLong("scroll_y", 0),
                tabIndex = o.optLong("tab_index", 0),
                articleHash = o.getString("article_hash"),
                generation = o.optLong("generation", 1),
            )
        }
    }
}

/** Reading-session UI state (the Kiwix tab). */
data class KiwixState(
    // -- the opened library + restored session --
    val zimPath: String = "",
    val zimOpen: Boolean = false,
    val zimUuid: String = "",
    val zimSizeBytes: Long = 0,
    val articlePath: String = "",
    val articleHtml: String? = null,
    val articleBytes: Int = 0,
    val articleHash: String = "",
    val scrollY: Long = 0,
    val generation: Long = 0,
    val strategy: KiwixStrategy = KiwixStrategy.PROGRESSIVE,
    val sessionId: String = "",
    val cutNumber: Long = 0,
    // -- carry (serve / receive) --
    val role: String = "", // "", "serve", "receive"
    val running: Boolean = false,
    val listenerWireAddr: String = "",
    val displayAddr: String = "",
    val ownPinHex: String = "",
    val console: List<String> = emptyList(),
    val bytesSent: Long = 0,
    val bytesRecv: Long = 0,
    /** MEASURED destination-side wire recv up to the first action (§5.6). */
    val bytesBeforeFirstAction: Long = 0,
    val mayMutate: Boolean? = null,
    val restoredArticlePath: String = "",
    val restoredScrollY: Long = 0,
    val evidenceBundlePath: String = "",
    val verifyOk: Boolean? = null,
    val error: String? = null,
    val runElapsedMs: Long = 0,
) {
    /** D/A in parts-per-million: bytes actually needed for the first action
     *  vs the whole library. The flagship number of this demo. */
    val ratioPpm: Long
        get() = KiwixMetrics.ratioPpm(bytesBeforeFirstAction, zimSizeBytes)
}

object KiwixRender {
    /** Text-first render of the CARRIED article bytes: hide <img> — assets live
     *  as separate ZIM entries and are not part of the carried object (the
     *  carry brings the position, not the book). Everything that came over the
     *  wire is still rendered; nothing is fetched from the network. */
    fun textFirstHtml(html: String): String {
        val style = "<style>img{display:none}.mw-ui-icon{display:none}</style>"
        val m = Regex("(?i)<head>").find(html)
        return if (m != null) {
            html.substring(0, m.range.last + 1) + style + html.substring(m.range.last + 1)
        } else {
            style + html
        }
    }
}

/** One ZIM asset (image, CSS, …) from the read-only reader.asset action
 *  (#25). The bytes are re-hashed here: a digest or size mismatch drops the
 *  asset (fail closed, the WebView gets a 404) instead of rendering it. */
class KiwixAsset(val path: String, val mimeType: String, val bytes: ByteArray) {
    companion object {
        fun parseActionResult(json: String): KiwixAsset? = try {
            val o = JSONObject(json).getJSONObject("output")
            val bytes = java.util.Base64.getDecoder().decode(o.getString("data_b64"))
            val want = o.getString("sha256").lowercase(java.util.Locale.ROOT)
            val got = java.security.MessageDigest.getInstance("SHA-256").digest(bytes)
                .joinToString("") { b -> "%02x".format(b.toInt() and 0xff) }
            val mime = o.optString("mimetype").substringBefore(';').trim()
            when {
                got != want -> null
                o.getLong("size") != bytes.size.toLong() -> null
                else -> KiwixAsset(
                    path = o.getString("path"),
                    mimeType = if (mime.isEmpty() || mime.any { it.isISOControl() || it.isWhitespace() }) {
                        "application/octet-stream"
                    } else {
                        mime
                    },
                    bytes = bytes,
                )
            }
        } catch (e: Exception) {
            null
        }
    }
}

/** URL <-> ZIM path mapping for the reader WebView (#25). Only
 *  https://kiwix.local/<zim path> is ours; every other URL is blocked. */
object KiwixAssets {
    const val HOST = "kiwix.local"

    /** Base URL for an article, so its relative asset links resolve to ZIM paths. */
    fun baseUrlFor(articlePath: String): String =
        java.net.URI("https", HOST, "/" + articlePath.trimStart('/'), null).toASCIIString()

    /** The ZIM path a WebView request asks for, or null when the URL is not ours. */
    fun zimPathFor(url: String): String? {
        val uri = try {
            java.net.URI(url)
        } catch (e: java.net.URISyntaxException) {
            return null
        }
        if (!uri.scheme.equals("https", ignoreCase = true)) return null
        if (!uri.host.equals(HOST, ignoreCase = true)) return null
        val path = uri.normalize().path?.removePrefix("/") ?: return null
        if (path.isEmpty() || path.split('/').any { it == ".." }) return null
        return path
    }
}

object KiwixMetrics {
    /** bytes_before_first_action / ZIM size, in ppm (integer, no float UI). */
    fun ratioPpm(carried: Long, zimSize: Long): Long =
        if (zimSize <= 0L || carried < 0L) 0L else (carried * 1_000_000L) / zimSize

    fun formatBytes(bytes: Long): String = when {
        bytes >= 1_000_000_000 -> String.format(java.util.Locale.US, "%.2f GB", bytes / 1e9)
        bytes >= 1_000_000 -> String.format(java.util.Locale.US, "%.1f MB", bytes / 1e6)
        bytes >= 1_000 -> String.format(java.util.Locale.US, "%.1f KB", bytes / 1e3)
        else -> "$bytes B"
    }

    fun formatRatio(ppm: Long): String =
        String.format(java.util.Locale.US, "%.4f%% (%d ppm)", ppm / 10_000.0, ppm)
}
