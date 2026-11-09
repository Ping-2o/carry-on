/*
 * KiwixScreen.kt — the Kiwix companion reader (stage 10, mobile half).
 *
 * The flagship story on one screen: a gigantic ZIM stays a LOCAL FILE; the
 * thing that moves between devices is the READING SESSION (article + scroll,
 * a few KB). `bytes_before_first_action` vs the ZIM size (D/A) is shown
 * prominently, and the wire counters/console prove it with MEASURED bytes.
 *
 * Roles (§2 labels):
 *   open     — device-local read of the real ZIM through libzim (OBSERVED),
 *              rendered in a WebView. Assets (images, CSS) are served from the
 *              LOCAL ZIM by the read-only reader.asset action through
 *              shouldInterceptRequest (#25); every other URL is blocked, and an
 *              asset whose digest does not match is dropped (fail closed). The
 *              carry still brings the position, not the book.
 *   serve    — this device offers its position to a peer (PHYSICAL when the
 *              peer is the mac, LOCAL on loopback).
 *   receive  — import the peer's position, restore it HERE, take authority,
 *              and move it on this device's CPU (L4).
 */
package com.carryon.app.ui.screens

import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.carryon.app.kiwix.KIWIX_ADAPTER_ID
import com.carryon.app.kiwix.KiwixAssets
import com.carryon.app.kiwix.KiwixMetrics
import com.carryon.app.kiwix.KiwixRender
import com.carryon.app.kiwix.KiwixRunner
import com.carryon.app.kiwix.KiwixStrategy
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.components.KeyValueLine
import com.carryon.app.ui.components.MonoConsole
import com.carryon.app.ui.theme.ConsoleTextStyle
import java.io.ByteArrayInputStream
import java.io.File

@Composable
fun KiwixScreen(runner: KiwixRunner = AppGraph.kiwixRunner) {
    val state by runner.state.collectAsState()
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current

    // The ZIM lives in the app's OWN external files dir: read without any
    // storage permission, and adb can push straight into it.
    val defaultZim = remember {
        File(context.getExternalFilesDir(null), "wikipedia_ru_mathematics_mini_2026-10.zim").absolutePath
    }
    val activity = LocalContext.current as? android.app.Activity
    val launch = remember { activity?.intent }
    var zimPath by remember { mutableStateOf(defaultZim) }
    var articlePath by remember { mutableStateOf(launch?.getStringExtra("kiwix_article") ?: "index") }
    var scrollY by remember { mutableStateOf((launch?.getStringExtra("kiwix_scroll") ?: "0").toLongOrNull() ?: 0L) }
    // Automation/testing aid (also a "paste connection info" affordance):
    // a launch Intent may pre-fill the peer coordinates and auto-run a role.
    //   adb shell am start -n com.carryon.app/.MainActivity \
    //     --es kiwix_open 1 --es kiwix_auto receive \
    //     --es kiwix_addr <ip:port> --es kiwix_peer_pin <64hex> \
    //     --es kiwix_session <uuid> --es kiwix_cut 0
    var peerPin by remember { mutableStateOf(launch?.getStringExtra("kiwix_peer_pin") ?: "") }
    var peerAddr by remember { mutableStateOf(launch?.getStringExtra("kiwix_addr") ?: "") }
    var remoteSession by remember { mutableStateOf(launch?.getStringExtra("kiwix_session") ?: "") }
    var remoteCut by remember { mutableStateOf((launch?.getStringExtra("kiwix_cut") ?: "0").toLongOrNull() ?: 0L) }
    val autoRole = remember { launch?.getStringExtra("kiwix_auto") }
    val autoOpen = remember { launch?.getStringExtra("kiwix_open") == "1" }
    // #25: render with assets from the local ZIM (default) or text-first.
    var withAssets by remember { mutableStateOf(true) }

    // One-shot auto-open (cold start with the Intent above).
    LaunchedEffect(Unit) {
        if (autoOpen && !state.zimOpen && !state.running) {
            runner.open(zimPath, articlePath, scrollY)
        }
    }
    // One-shot auto-role once the library is open (serve or receive).
    LaunchedEffect(state.zimOpen, autoRole) {
        if (state.zimOpen && autoRole != null && !state.running) {
            when (autoRole) {
                "receive" -> runner.receive(peerAddr, peerPin, remoteSession, remoteCut, articlePath, scrollY)
                "serve" -> runner.serve(peerPin)
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Kiwix reader", style = MaterialTheme.typography.headlineSmall)
        Text(
            "One reading session over a gigantic ZIM — the library stays local, the position moves.\n" +
                "OBSERVED per run (LOCAL on loopback, PHYSICAL across two machines); platform supported: NO (§30 unrun).",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.tertiary,
        )

        // ---- library ----
        Text("Library (ZIM opens locally through libzim)", style = MaterialTheme.typography.titleSmall)
        OutlinedTextField(
            value = zimPath,
            onValueChange = {
                zimPath = it
                runner.setZimPath(it)
            },
            label = { Text("ZIM path (app files dir; adb push target)") },
            modifier = Modifier.fillMaxWidth(),
            enabled = !state.running,
            singleLine = true,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = articlePath,
                onValueChange = { articlePath = it },
                label = { Text("article path") },
                modifier = Modifier.weight(1f),
                enabled = !state.running,
                singleLine = true,
            )
            OutlinedTextField(
                value = scrollY.toString(),
                onValueChange = { v -> v.toLongOrNull()?.let { scrollY = it } },
                label = { Text("scroll_y") },
                modifier = Modifier.weight(0.6f),
                enabled = !state.running,
                singleLine = true,
            )
        }
        // Sample entries of the math-mini ZIM (real paths) + position presets:
        // tap-driven so the demo needs no keyboard (adb-friendly).
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf("index", "Mathematics", "Физика", "Эйнштейн,_Альберт").forEach { preset ->
                FilterChip(
                    selected = articlePath == preset,
                    onClick = { articlePath = preset },
                    label = { Text(preset.take(14)) },
                    enabled = !state.running,
                )
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf(0L, 128L, 321L).forEach { preset ->
                FilterChip(
                    selected = scrollY == preset,
                    onClick = { scrollY = preset },
                    label = { Text("scroll $preset") },
                    enabled = !state.running,
                )
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KiwixStrategy.entries.forEach { s ->
                FilterChip(
                    selected = state.strategy == s,
                    onClick = { runner.setStrategy(s) },
                    label = { Text(s.code) },
                    enabled = !state.running,
                )
            }
        }
        Text(
            "D = progressive (the ZIM bulk stays an unsealed Cache object — the cut is KBs);\n" +
                "A = bulk sealed up front (what NOT to do with a 27 MB–1.7 GB library).",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        Button(
            onClick = { runner.open(zimPath, articlePath, scrollY) },
            enabled = !state.running && zimPath.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) { Text(if (state.running) "WORKING…" else "▤ OPEN ZIM + RESTORE READING") }

        state.error?.let {
            Text("FAIL: $it", color = MaterialTheme.colorScheme.error, style = ConsoleTextStyle)
        }

        // ---- the article (real carried/restored bytes) ----
        Text("Article", style = MaterialTheme.typography.titleSmall)
        state.articleHtml?.let { html ->
            FilterChip(
                selected = withAssets,
                onClick = { withAssets = !withAssets },
                label = { Text(if (withAssets) "assets: from the local ZIM" else "assets: hidden (text-first)") },
            )
            AndroidView(
                factory = { ctx ->
                    WebView(ctx).apply {
                        settings.javaScriptEnabled = false // carried bytes are DATA (EVD-005)
                        settings.allowFileAccess = false
                        settings.allowContentAccess = false
                        settings.loadWithOverviewMode = true
                        settings.useWideViewPort = true
                        setBackgroundColor(0xFFFFFFFF.toInt())
                        webViewClient = KiwixAssetClient(runner)
                    }
                },
                update = { wv ->
                    wv.loadDataWithBaseURL(
                        KiwixAssets.baseUrlFor(state.articlePath),
                        if (withAssets) html else KiwixRender.textFirstHtml(html),
                        "text/html",
                        "utf-8",
                        null,
                    )
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(340.dp),
            )
        } ?: Text(
            "open the ZIM to render an article (the engine loads it through libzim)",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.outline,
        )
        if (state.zimOpen) {
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                KeyValueLine("article", state.articlePath)
                KeyValueLine("bytes", KiwixMetrics.formatBytes(state.articleBytes.toLong()))
            }
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                KeyValueLine("scroll_y", state.scrollY.toString())
                KeyValueLine("generation", state.generation.toString())
            }
            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                KeyValueLine("cut", state.cutNumber.toString())
                KeyValueLine("zim_size", KiwixMetrics.formatBytes(state.zimSizeBytes))
            }
        }

        // ---- position move on this device (owner, L4) ----
        if (state.zimOpen) {
            Text("Move the position on THIS device (reader.navigate — mutating, L4)", style = MaterialTheme.typography.titleSmall)
            Button(
                onClick = { runner.navigate(articlePath, scrollY) },
                enabled = !state.running,
                modifier = Modifier.fillMaxWidth(),
            ) { Text("⤓ NAVIGATE + RE-SEAL CUT") }
        }

        // ---- serve this position to a peer ----
        Text("Serve to a peer (LAN TLS 1.3, mutual pin §21.2)", style = MaterialTheme.typography.titleSmall)
        OutlinedTextField(
            value = peerPin,
            onValueChange = { peerPin = it.trim().lowercase() },
            label = { Text("peer pin (64 hex)") },
            modifier = Modifier.fillMaxWidth(),
            enabled = !state.running,
            singleLine = true,
        )
        Button(
            onClick = { runner.serve(peerPin) },
            enabled = !state.running && state.zimOpen && peerPin.length == 64,
            modifier = Modifier.fillMaxWidth(),
        ) { Text(if (state.role == "serve" && state.running) "SERVING… (waiting for the peer)" else "▲ SERVE POSITION") }
        if (state.displayAddr.isNotBlank()) {
            KeyValueLine("listen", state.displayAddr)
            SelectionContainer {
                Text("own_pin:  ${state.ownPinHex}", style = ConsoleTextStyle)
            }
            KeyValueLine("session", state.sessionId)
            KeyValueLine("cut", state.cutNumber.toString())
            OutlinedButton(onClick = { clipboard.setText(AnnotatedString(state.ownPinHex)) }) {
                Text("Copy own pin")
            }
        }

        // ---- receive a position from a peer ----
        Text("Receive a position from a peer", style = MaterialTheme.typography.titleSmall)
        OutlinedTextField(
            value = peerAddr,
            onValueChange = { peerAddr = it.trim() },
            label = { Text("source addr (ip:port)") },
            modifier = Modifier.fillMaxWidth(),
            enabled = !state.running,
            singleLine = true,
        )
        OutlinedTextField(
            value = remoteSession,
            onValueChange = { remoteSession = it.trim() },
            label = { Text("source session id") },
            modifier = Modifier.fillMaxWidth(),
            enabled = !state.running,
            singleLine = true,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = remoteCut.toString(),
                onValueChange = { v -> v.toLongOrNull()?.let { remoteCut = it } },
                label = { Text("cut") },
                modifier = Modifier.weight(0.5f),
                enabled = !state.running,
                singleLine = true,
            )
            OutlinedTextField(
                value = peerPin.ifBlank { "" },
                onValueChange = { peerPin = it.trim().lowercase() },
                label = { Text("source pin (64 hex)") },
                modifier = Modifier.weight(1f),
                enabled = !state.running,
                singleLine = true,
            )
        }
        Button(
            onClick = { runner.receive(peerAddr, peerPin, remoteSession, remoteCut, articlePath, scrollY) },
            enabled = !state.running && state.zimOpen &&
                peerAddr.isNotBlank() && peerPin.length == 64 && remoteSession.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) { Text(if (state.role == "receive" && state.running) "RECEIVING…" else "▼ RECEIVE + RESTORE HERE") }

        // ---- the flagship metric: gigantic ZIM vs tiny position ----
        Text("bytes_before_first_action (§5.6 — MEASURED wire bytes)", style = MaterialTheme.typography.titleSmall)
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            KeyValueLine("wire.sent", KiwixMetrics.formatBytes(state.bytesSent))
            KeyValueLine("wire.recv", KiwixMetrics.formatBytes(state.bytesRecv))
        }
        KeyValueLine("bytes_before_first_action", KiwixMetrics.formatBytes(state.bytesBeforeFirstAction))
        if (state.zimSizeBytes > 0) {
            KeyValueLine(
                "D/A",
                "${KiwixMetrics.formatRatio(state.ratioPpm)} of ${KiwixMetrics.formatBytes(state.zimSizeBytes)} (ZIM stays local)",
            )
        }
        state.mayMutate?.let { KeyValueLine("may_mutate", it.toString()) }
        state.evidenceBundlePath.takeIf { it.isNotBlank() }?.let {
            KeyValueLine("evidence", it)
            KeyValueLine("verify_evidence", "ok=${state.verifyOk}")
        }

        if (state.console.isNotEmpty()) {
            MonoConsole(lines = state.console, footer = null)
        }
        Text(
            text = "Adapter id: $KIWIX_ADAPTER_ID (compiled-in; needs an engine built with --features kiwix).\n" +
                "The article bytes are read from the engine's content-addressed store and sha256-verified\n" +
                "by the core before render (CORE-004). The store's temporary dirs live in cacheDir.",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        Spacer(Modifier.height(24.dp))
    }
}

/** Serves https://kiwix.local/<path> from the local ZIM (reader.asset, #25)
 *  and blocks every other request: nothing is fetched from the network, and
 *  links do not navigate (the position moves only through reader.navigate, L4). */
private class KiwixAssetClient(private val runner: KiwixRunner) : WebViewClient() {
    override fun shouldInterceptRequest(view: WebView?, request: WebResourceRequest?): WebResourceResponse? {
        val url = request?.url?.toString() ?: return emptyResponse(403, "Blocked")
        val path = KiwixAssets.zimPathFor(url) ?: return emptyResponse(403, "Blocked")
        val asset = runner.readAsset(path) ?: return emptyResponse(404, "Not in ZIM")
        return WebResourceResponse(asset.mimeType, null, ByteArrayInputStream(asset.bytes))
    }

    override fun shouldOverrideUrlLoading(view: WebView?, request: WebResourceRequest?): Boolean = true

    private fun emptyResponse(code: Int, reason: String): WebResourceResponse =
        WebResourceResponse("text/plain", "utf-8", code, reason, emptyMap(), ByteArrayInputStream(ByteArray(0)))
}
