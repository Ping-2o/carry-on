/*
 * QrScannerView.kt — CameraX preview + frame analysis for pairing QR scans
 * (issue #26). Frames are decoded off the main thread with zxing
 * (QrDecoder); the first decoded text is delivered ONCE on the main thread,
 * then the caller removes this view, which unbinds the camera.
 *
 * The caller must already hold the CAMERA permission (PairingScreen requests
 * it only when the user taps "Scan QR"). Nothing here validates the payload;
 * that is the engine's job (QrScan.kt).
 *
 * Labels (§2): TARGET until a device run is recorded.
 */
package com.carryon.app.ui.screens

import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.carryon.app.pairing.QrDecoder
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

@Composable
fun QrScannerView(
    onScanned: (String) -> Unit,
    onError: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val previewView = remember {
        // TextureView-backed: behaves inside a scrolling Compose column.
        PreviewView(context).apply { implementationMode = PreviewView.ImplementationMode.COMPATIBLE }
    }
    val latestOnScanned by rememberUpdatedState(onScanned)
    val latestOnError by rememberUpdatedState(onError)

    DisposableEffect(lifecycleOwner) {
        val analysisExecutor = Executors.newSingleThreadExecutor()
        val mainExecutor = ContextCompat.getMainExecutor(context)
        val delivered = AtomicBoolean(false)
        val providerFuture = ProcessCameraProvider.getInstance(context)
        var provider: ProcessCameraProvider? = null
        var disposed = false

        providerFuture.addListener({
            if (disposed) return@addListener
            try {
                val p = providerFuture.get()
                provider = p
                val preview = Preview.Builder().build()
                preview.setSurfaceProvider(previewView.surfaceProvider)
                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(analysisExecutor) { image ->
                    val text = try {
                        if (delivered.get()) null else decodeFrame(image)
                    } finally {
                        image.close()
                    }
                    if (text != null && delivered.compareAndSet(false, true)) {
                        mainExecutor.execute { latestOnScanned(text) }
                    }
                }
                p.unbindAll()
                p.bindToLifecycle(lifecycleOwner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            } catch (e: Exception) {
                latestOnError(e.message ?: "camera unavailable")
            }
        }, mainExecutor)

        onDispose {
            disposed = true
            provider?.unbindAll()
            analysisExecutor.shutdown()
        }
    }

    AndroidView(factory = { previewView }, modifier = modifier)
}

/** The Y (luminance) plane of a YUV_420_888 frame, copied with its row stride. */
private fun decodeFrame(image: ImageProxy): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer.duplicate()
    val rowStride = plane.rowStride
    val height = image.height
    val bytes = ByteArray(rowStride * height)
    buffer.get(bytes, 0, minOf(buffer.remaining(), bytes.size))
    return QrDecoder.decodeLuminance(bytes, image.width, height, rowStride)
}
