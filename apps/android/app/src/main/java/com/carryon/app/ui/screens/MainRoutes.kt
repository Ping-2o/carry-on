/*
 * MainRoutes.kt — thin stateful wrappers: collect live engine/topology state
 * from AppGraph, map it to UI models, and hand it to the stateless content.
 */
package com.carryon.app.ui.screens

import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import com.carryon.app.discovery.DiscoveredPeer
import com.carryon.app.handoff.ReceiveState
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.SessionRow
import com.carryon.app.ui.model.ActivityRole
import com.carryon.app.ui.model.ActivityUi
import com.carryon.app.ui.model.DeviceKind
import com.carryon.app.ui.model.DeviceStatus
import com.carryon.app.ui.model.DeviceUi
import com.carryon.app.ui.model.DevicesUiState
import com.carryon.app.ui.model.HomeUiState
import com.carryon.app.ui.model.ReceivePhase
import com.carryon.app.ui.model.ReceiveUiState
import dev.carryon.kit.TrustRecord
import kotlinx.coroutines.launch

fun DiscoveredPeer.toUi(): DeviceUi = DeviceUi(
    id = instanceId,
    name = serviceName,
    kind = DeviceKind.fromDeviceClass(deviceClass),
    detail = if (pairingRequired) "Pairing required · $host" else "Available · $host",
    status = DeviceStatus.Nearby,
    address = "$host:$port",
)

fun TrustRecord.toUi(): DeviceUi = DeviceUi(
    id = pinHex,
    name = deviceName,
    kind = DeviceKind.guess(deviceName),
    detail = if (revoked) "Revoked · pin $shortPin" else "Pin $shortPin · gen $trustGeneration",
    status = if (revoked) DeviceStatus.Revoked else DeviceStatus.Paired,
)

fun SessionRow.toUi(): ActivityUi {
    val r = role.lowercase()
    val activityRole = when {
        "demo" in r -> ActivityRole.Demo
        "target" in r || "receiv" in r || "mirror" in r -> ActivityRole.Received
        "source" in r || "send" in r -> ActivityRole.Sent
        else -> ActivityRole.Prepared
    }
    val cut = cutNumber?.let { "cut $it" }
    val epoch = authorityEpoch?.let { "epoch $it" }
    return ActivityUi(
        id = sessionId,
        title = title.ifBlank { sessionId.take(12) },
        subtitle = listOfNotNull(role, cut, epoch).joinToString(" · "),
        state = state,
        role = activityRole,
        whenLabel = "",
    )
}

fun ReceiveState.toUi(deviceName: String, advertising: Boolean, advertisedName: String): ReceiveUiState {
    val result = lastResult
    val phase = when {
        running -> ReceivePhase.Receiving
        error != null -> ReceivePhase.Failed
        result != null && result.ok -> ReceivePhase.Done
        result != null -> ReceivePhase.Failed
        armed -> ReceivePhase.Waiting
        else -> ReceivePhase.Arming
    }
    return ReceiveUiState(
        phase = phase,
        deviceName = deviceName,
        address = displayAddr,
        shortPin = shortPin,
        fullPin = pinHex,
        advertising = advertising,
        advertisedName = advertisedName,
        trustedCount = pairedRecords,
        sessionState = sessionState,
        bytesReceived = bytesRecv,
        elapsedMs = runElapsedMs,
        verified = result?.verifyOk,
        error = error ?: if (result?.refused == true) "The integrity check refused the bytes. Nothing was opened." else null,
        console = console,
    )
}

@Composable
fun HomeRoute(onNavigate: (String) -> Unit) {
    val topology = AppGraph.topology
    LaunchedEffect(Unit) { topology.ensureLoaded() }
    val identity by topology.identity.collectAsState()
    val trust by topology.trust.collectAsState()
    val sessions by topology.sessions.collectAsState()
    val discovery by AppGraph.discovery.state.collectAsState()
    val receive by AppGraph.receiveRunner.state.collectAsState()
    val recent = sessions.asReversed().map { it.toUi() }
    HomeContent(
        state = HomeUiState(
            deviceName = identity?.deviceName ?: Build.MODEL,
            shortPin = identity?.shortPin ?: "…",
            receiving = receive.armed,
            nearby = discovery.peers.map { it.toUi() },
            trustedCount = trust.count { !it.revoked },
            recent = recent.take(3),
            transfersToday = sessions.size,
        ),
        onReceive = { onNavigate("receive") },
        onDevices = { onNavigate("devices") },
        onActivity = { onNavigate("activity") },
        onDemo = { onNavigate("demo") },
    )
}

@Composable
fun ReceiveRoute(onNavigate: (String) -> Unit) {
    val runner = AppGraph.receiveRunner
    val state by runner.state.collectAsState()
    val discovery by AppGraph.discovery.state.collectAsState()
    val identity by AppGraph.topology.identity.collectAsState()
    val clipboard = LocalClipboardManager.current

    // Same lifecycle as the engine console screen: arm once, advertise the
    // armed port over NSD, stop advertising when the screen leaves (battery).
    LaunchedEffect(Unit) { runner.arm() }
    LaunchedEffect(state.displayAddr) {
        val port = state.displayAddr.substringAfterLast(':').toIntOrNull()
        if (state.armed && port != null && port > 0) AppGraph.discovery.advertise(port = port)
    }
    DisposableEffect(Unit) { onDispose { AppGraph.discovery.stop() } }

    ReceiveContent(
        state = state.toUi(
            deviceName = identity?.deviceName ?: Build.MODEL,
            advertising = discovery.advertising,
            advertisedName = discovery.advertisedName ?: "",
        ),
        onAccept = { runner.accept(null, 0L) },
        onRearm = { runner.arm() },
        onCopyPin = { clipboard.setText(AnnotatedString(state.pinHex)) },
        onOpenConsole = { onNavigate("receive/console") },
    )
}

@Composable
fun DevicesRoute(onNavigate: (String) -> Unit) {
    val topology = AppGraph.topology
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) { topology.ensureLoaded() }
    val identity by topology.identity.collectAsState()
    val trust by topology.trust.collectAsState()
    val discovery by AppGraph.discovery.state.collectAsState()
    DisposableEffect(Unit) {
        AppGraph.discovery.startBrowse()
        onDispose { AppGraph.discovery.stop() }
    }
    val name = identity?.deviceName ?: Build.MODEL
    DevicesContent(
        state = DevicesUiState(
            thisDevice = DeviceUi(
                id = "self",
                name = name,
                kind = DeviceKind.Phone,
                detail = "Pin ${identity?.shortPin ?: "…"} · Android ${Build.VERSION.RELEASE}",
                status = DeviceStatus.ThisDevice,
                address = identity?.listenerAddr,
            ),
            browsing = discovery.browsing,
            nearby = discovery.peers.map { it.toUi() },
            trusted = trust.map { it.toUi() },
            firstPeerLatencyMs = discovery.firstPeerLatencyMs,
        ),
        onPair = { onNavigate("pairing") },
        onRevoke = { pin -> scope.launch { topology.revoke(pin) } },
    )
}

@Composable
fun ActivityRoute() {
    val topology = AppGraph.topology
    LaunchedEffect(Unit) { topology.ensureLoaded() }
    val sessions by topology.sessions.collectAsState()
    ActivityContent(sessions.asReversed().map { it.toUi() })
}
