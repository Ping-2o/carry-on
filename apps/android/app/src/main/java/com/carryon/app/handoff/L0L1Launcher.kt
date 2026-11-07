/*
 * L0L1Launcher.kt — the platform half of L1 launcher activation (§10.3
 * PLAT-003), the app-level version of appcont_handoff.sh's launch step: the
 * verified bytes go into MediaStore (scoped-storage insert, no permission on
 * API 29+ for app-owned entries; RELATIVE_PATH mirrors the shell's
 * /sdcard/Download + media-scanner flow), then a real ACTION_VIEW intent with
 * the exact mime resolves and launches a REAL installed application.
 *
 * Honest label (§2): `launch` reports the resolved activity or null — the
 * caller never claims "opened in a real app" without a real resolution.
 */
package com.carryon.app.handoff

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore

object L0L1Launcher {

    /** Insert the verified bytes into MediaStore and return the content URI.
     *  Collection by mime (images → Pictures/Carry-On, else Downloads, etc.). */
    fun writeToMediaStore(context: Context, bytes: ByteArray, fileName: String, mime: String): Uri {
        val resolver = context.contentResolver
        val (collection, relativePath) = collectionFor(mime)
        val values = android.content.ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, fileName)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.RELATIVE_PATH, relativePath)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(collection, values)
            ?: throw IllegalStateException("MediaStore insert returned null")
        resolver.openOutputStream(uri)?.use { it.write(bytes) }
            ?: throw IllegalStateException("MediaStore output stream unavailable for $uri")
        values.clear()
        values.put(MediaStore.MediaColumns.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
        return uri
    }

    /** Resolve + launch ACTION_VIEW for the carried file. Returns the resolved
     *  activity (flattened component) or null when nothing handles the mime —
     *  the honest "carried + verified, NOT opened" case. */
    fun launch(context: Context, uri: Uri, mime: String): String? {
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, mime)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        val resolved = context.packageManager.resolveActivity(intent, 0)?.activityInfo
            ?.let { "${it.packageName}/${it.name}" }
            ?: return null
        context.startActivity(intent)
        return resolved
    }

    /** (collection, relativePath) by mime — small, honest, unit-testable. */
    fun collectionFor(mime: String): Pair<Uri, String> = when {
        mime.startsWith("image/") -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI to "Pictures/Carry-On"
        mime.startsWith("video/") -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI to "Movies/Carry-On"
        mime.startsWith("audio/") -> MediaStore.Audio.Media.EXTERNAL_CONTENT_URI to "Music/Carry-On"
        else -> MediaStore.Downloads.EXTERNAL_CONTENT_URI to "Download/Carry-On"
    }
}
