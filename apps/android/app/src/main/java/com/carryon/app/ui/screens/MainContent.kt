/*
 * MainContent.kt — the production screens, as STATELESS content composables.
 * They take a UI model + callbacks only (no AppGraph, no JNI), so the same code
 * renders live (MainRoutes.kt) and in Roborazzi screenshots (ScreenshotTest).
 */
package com.carryon.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.AutoStories
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Fingerprint
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Hub
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Inventory2
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Science
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Timeline
import androidx.compose.material.icons.outlined.Upload
import androidx.compose.material.icons.outlined.Verified
import androidx.compose.material.icons.outlined.WifiTethering
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.carryon.app.ui.design.ActivityListItem
import com.carryon.app.ui.design.DeviceListItem
import com.carryon.app.ui.design.EmptyState
import com.carryon.app.ui.design.HeroSurface
import com.carryon.app.ui.design.HonestyLabel
import com.carryon.app.ui.design.IconBadge
import com.carryon.app.ui.design.KeyValue
import com.carryon.app.ui.design.ListDivider
import com.carryon.app.ui.design.MetricRow
import com.carryon.app.ui.design.MetricTile
import com.carryon.app.ui.design.NavRow
import com.carryon.app.ui.design.SectionCard
import com.carryon.app.ui.design.Space
import com.carryon.app.ui.design.StatusChip
import com.carryon.app.ui.design.TechnicalDetails
import com.carryon.app.ui.model.ActivityRole
import com.carryon.app.ui.model.ActivityUi
import com.carryon.app.ui.model.DeviceStatus
import com.carryon.app.ui.model.DevicesUiState
import com.carryon.app.ui.model.HomeUiState
import com.carryon.app.ui.model.ReceivePhase
import com.carryon.app.ui.model.ReceiveUiState
import com.carryon.app.ui.model.SessionStates
import com.carryon.app.ui.model.Tone
import com.carryon.app.ui.model.formatBytes
import com.carryon.app.ui.model.formatMs
import com.carryon.app.ui.theme.ConsoleTextStyle

// ---------------------------------------------------------------- scaffolds

/** Top-level destination: large collapsing app bar + centered, width-capped list. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TopLevelScaffold(
    title: String,
    actions: @Composable RowScope.() -> Unit = {},
    floatingActionButton: @Composable () -> Unit = {},
    content: LazyListScope.() -> Unit,
) {
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        containerColor = MaterialTheme.colorScheme.surface,
        topBar = {
            LargeTopAppBar(
                title = { Text(title) },
                actions = actions,
                scrollBehavior = scroll,
                colors = TopAppBarDefaults.largeTopAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
            )
        },
        floatingActionButton = floatingActionButton,
    ) { padding ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
            LazyColumn(
                modifier = Modifier.widthIn(max = 840.dp).fillMaxSize(),
                contentPadding = PaddingValues(
                    start = Space.l,
                    end = Space.l,
                    top = padding.calculateTopPadding() + Space.s,
                    bottom = padding.calculateBottomPadding() + 96.dp,
                ),
                verticalArrangement = Arrangement.spacedBy(Space.m),
                content = content,
            )
        }
    }
}

/** Pushed (sub) destination with a back arrow; hosts legacy developer screens too. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DetailScaffold(title: String, onBack: (() -> Unit)?, content: @Composable () -> Unit) {
    Scaffold(
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        containerColor = MaterialTheme.colorScheme.surface,
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) { content() }
    }
}

private fun ActivityRole.icon(): ImageVector = when (this) {
    ActivityRole.Received -> Icons.Outlined.Download
    ActivityRole.Sent -> Icons.Outlined.Upload
    ActivityRole.Prepared -> Icons.Outlined.Inventory2
    ActivityRole.Demo -> Icons.Outlined.Science
}

// --------------------------------------------------------------------- home

@Composable
fun HomeContent(
    state: HomeUiState,
    onReceive: () -> Unit,
    onDevices: () -> Unit,
    onActivity: () -> Unit,
    onDemo: () -> Unit,
) {
    TopLevelScaffold(title = "Carry-On") {
        item(key = "hero") {
            HeroSurface {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    StatusChip(
                        if (state.receiving) "Receiving" else "Not receiving",
                        if (state.receiving) Tone.Success else Tone.Neutral,
                        icon = if (state.receiving) Icons.Outlined.WifiTethering else null,
                    )
                }
                Text(
                    "Pick up right where you left off",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
                Text(
                    "Send a file or session from a trusted Mac — it arrives here verified, byte for byte.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.85f),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(Space.s)) {
                    Button(onClick = onReceive) {
                        Icon(Icons.Outlined.Download, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(Space.s))
                        Text(if (state.receiving) "Open Receive" else "Start receiving")
                    }
                    FilledTonalButton(onClick = onDemo) { Text("Try the demo") }
                }
            }
        }
        item(key = "metrics") {
            MetricRow {
                MetricTile("Trusted", "${state.trustedCount}", Icons.Outlined.Verified, Modifier.weight(1f), Tone.Success)
                MetricTile("Nearby", "${state.nearby.size}", Icons.Outlined.WifiTethering, Modifier.weight(1f), Tone.Info)
                MetricTile("Today", "${state.transfersToday}", Icons.Outlined.History, Modifier.weight(1f), Tone.Neutral)
            }
        }
        item(key = "nearby") {
            SectionCard(
                title = "Nearby",
                icon = Icons.Outlined.WifiTethering,
                contentPadding = Space.s,
                action = { TextButton(onClick = onDevices) { Text("All devices") } },
            ) {
                if (state.nearby.isEmpty()) {
                    EmptyState(Icons.Outlined.Devices, "No devices nearby", "Open Carry-On on your Mac on the same Wi-Fi.")
                } else {
                    state.nearby.forEachIndexed { i, d ->
                        if (i > 0) ListDivider()
                        DeviceListItem(d, onClick = onDevices)
                    }
                }
            }
        }
        item(key = "recent") {
            SectionCard(
                title = "Recent activity",
                icon = Icons.Outlined.History,
                contentPadding = Space.s,
                action = { TextButton(onClick = onActivity) { Text("See all") } },
            ) {
                if (state.recent.isEmpty()) {
                    EmptyState(Icons.Outlined.History, "Nothing yet", "Transfers you receive show up here.")
                } else {
                    state.recent.forEachIndexed { i, a ->
                        if (i > 0) ListDivider()
                        ActivityListItem(a, a.role.icon())
                    }
                }
            }
        }
        item(key = "label") {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                HonestyLabel("${state.deviceName} · pin ${state.shortPin} · Demo · OBSERVED (LOCAL)")
            }
        }
    }
}

// ------------------------------------------------------------------ receive

@Composable
fun ReceiveContent(
    state: ReceiveUiState,
    onAccept: () -> Unit,
    onRearm: () -> Unit,
    onCopyPin: () -> Unit,
    onOpenConsole: () -> Unit,
) {
    TopLevelScaffold(
        title = "Receive",
        actions = {
            IconButton(onClick = onRearm, enabled = state.phase != ReceivePhase.Receiving) {
                Icon(Icons.Outlined.Refresh, contentDescription = "Restart listener")
            }
        },
    ) {
        item(key = "hero") {
            val (icon, tone, title, body) = when (state.phase) {
                ReceivePhase.Arming -> Quad(Icons.Outlined.WifiTethering, Tone.Neutral, "Getting ready…", "Opening a secure listener on this phone.")
                ReceivePhase.Waiting -> Quad(
                    Icons.Outlined.WifiTethering, Tone.Info, "Waiting for a trusted device",
                    if (state.trustedCount > 0) "On your Mac, choose Send and pick ${state.deviceName}."
                    else "Pair a device first — then transfers need no typing.",
                )
                ReceivePhase.Receiving -> Quad(Icons.Outlined.Download, Tone.Info, "Receiving…", "Bytes are verified as they arrive.")
                ReceivePhase.Done -> Quad(
                    Icons.Outlined.CheckCircle, Tone.Success, "Received and verified",
                    SessionStates.explanation(state.sessionState ?: "ActionReady"),
                )
                ReceivePhase.Failed -> Quad(Icons.Outlined.ErrorOutline, Tone.Danger, "Stopped safely", state.error ?: "Nothing was opened.")
            }
            HeroSurface {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Space.m)) {
                    IconBadge(icon, tone, size = 48.dp)
                    Column(Modifier.weight(1f)) {
                        Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onPrimaryContainer)
                        Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.85f))
                    }
                }
                if (state.phase == ReceivePhase.Receiving || state.phase == ReceivePhase.Arming) {
                    LinearProgressIndicator(Modifier.fillMaxWidth())
                }
                when (state.phase) {
                    ReceivePhase.Waiting -> Button(onClick = onAccept, enabled = state.trustedCount > 0) {
                        Icon(Icons.Outlined.Lock, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(Space.s))
                        Text("Accept from trusted device")
                    }
                    ReceivePhase.Done, ReceivePhase.Failed -> FilledTonalButton(onClick = onRearm) { Text("Receive another") }
                    else -> {}
                }
            }
        }
        if (state.phase == ReceivePhase.Done) {
            item(key = "result") {
                MetricRow {
                    MetricTile("Received", formatBytes(state.bytesReceived), Icons.Outlined.Download, Modifier.weight(1f))
                    MetricTile("Time", formatMs(state.elapsedMs), Icons.Outlined.Speed, Modifier.weight(1f))
                    MetricTile(
                        "Integrity",
                        if (state.verified == true) "Verified" else "Unproven",
                        Icons.Outlined.Verified,
                        Modifier.weight(1f),
                        if (state.verified == true) Tone.Success else Tone.Warning,
                    )
                }
            }
        }
        item(key = "phone") {
            SectionCard(
                title = "This phone",
                icon = Icons.Outlined.Fingerprint,
                action = {
                    StatusChip(
                        if (state.advertising) "Visible nearby" else "Not visible",
                        if (state.advertising) Tone.Success else Tone.Neutral,
                    )
                },
            ) {
                KeyValue("Name", state.deviceName)
                KeyValue("Address", state.address.ifBlank { "…" }, mono = true)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.weight(1f)) { KeyValue("Pin", state.shortPin.ifBlank { "…" }, mono = true) }
                    IconButton(onClick = onCopyPin, enabled = state.fullPin.isNotBlank()) {
                        Icon(Icons.Outlined.ContentCopy, contentDescription = "Copy full pin")
                    }
                }
                Text(
                    "The pin is this phone’s public identity fingerprint — not a secret.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        item(key = "tech") {
            TechnicalDetails {
                KeyValue("Session", state.sessionState ?: "—", mono = true)
                KeyValue("Trusted", "${state.trustedCount}")
                KeyValue("NSD name", state.advertisedName.ifBlank { "—" }, mono = true)
                state.console.takeLast(8).forEach { Text(it, style = ConsoleTextStyle) }
                OutlinedButton(onClick = onOpenConsole) {
                    Icon(Icons.Outlined.Terminal, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(Space.s))
                    Text("Open engine console")
                }
            }
        }
        item(key = "label") {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { HonestyLabel() }
        }
    }
}

private data class Quad(val icon: ImageVector, val tone: Tone, val title: String, val body: String)

// ------------------------------------------------------------------ devices

@Composable
fun DevicesContent(state: DevicesUiState, onPair: () -> Unit, onRevoke: (String) -> Unit) {
    TopLevelScaffold(
        title = "Devices",
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = onPair,
                icon = { Icon(Icons.Outlined.Add, contentDescription = null) },
                text = { Text("Pair a device") },
            )
        },
    ) {
        item(key = "self") {
            SectionCard(contentPadding = Space.s) {
                DeviceListItem(state.thisDevice)
            }
        }
        item(key = "nearby") {
            SectionCard(
                title = "Nearby",
                icon = Icons.Outlined.WifiTethering,
                contentPadding = Space.s,
                action = {
                    StatusChip(
                        if (state.browsing) "Looking…" else "Paused",
                        if (state.browsing) Tone.Info else Tone.Neutral,
                    )
                },
            ) {
                if (state.nearby.isEmpty()) {
                    EmptyState(Icons.Outlined.WifiTethering, "No devices found", "Make sure both devices are on the same Wi-Fi.")
                } else {
                    state.nearby.forEachIndexed { i, d ->
                        if (i > 0) ListDivider()
                        DeviceListItem(d)
                    }
                }
                state.firstPeerLatencyMs?.let {
                    Text(
                        "First device found in ${formatMs(it)}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = Space.l, vertical = Space.xs),
                    )
                }
            }
        }
        item(key = "trusted") {
            SectionCard(title = "Trusted devices", icon = Icons.Outlined.Verified, contentPadding = Space.s) {
                if (state.trusted.isEmpty()) {
                    EmptyState(Icons.Outlined.Key, "No trusted devices", "Pair once; after that, transfers need no typing.")
                } else {
                    state.trusted.forEachIndexed { i, d ->
                        if (i > 0) ListDivider()
                        DeviceListItem(
                            d,
                            trailing = {
                                if (d.status == DeviceStatus.Paired) {
                                    TextButton(onClick = { onRevoke(d.id) }) { Text("Revoke") }
                                } else {
                                    StatusChip(d.status.label, d.status.tone)
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

// --------------------------------------------------------------------- more

@Composable
fun MoreContent(onOpen: (String) -> Unit, versionName: String) {
    TopLevelScaffold(title = "More") {
        item(key = "activity") {
            SectionCard(contentPadding = Space.s) {
                NavRow("Activity", "Every session and transfer, with its exact state", Icons.Outlined.History, { onOpen("activity") }, Tone.Info)
                ListDivider()
                NavRow("Offline library", "Carry Kiwix archives between devices", Icons.Outlined.AutoStories, { onOpen("kiwix") }, Tone.Info)
            }
        }
        item(key = "devhdr") {
            Text(
                "Developer",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = Space.s, top = Space.s),
            )
        }
        item(key = "dev") {
            SectionCard(contentPadding = Space.s) {
                NavRow("Engine demo", "Run the loopback handoff end to end", Icons.Outlined.Science, { onOpen("demo") })
                ListDivider()
                NavRow("Receive console", "Raw listener, ladder and manual pin fallback", Icons.Outlined.Terminal, { onOpen("receive/console") })
                ListDivider()
                NavRow("Pairing", "QR + six-digit code ceremony", Icons.Outlined.Key, { onOpen("pairing") })
                ListDivider()
                NavRow("Topology", "Identity, trust records and sessions", Icons.Outlined.Hub, { onOpen("topology") })
                ListDivider()
                NavRow("Sessions", "Session table (engine view)", Icons.Outlined.Timeline, { onOpen("sessions") })
                ListDivider()
                NavRow("Evidence", "Signed evidence bundles", Icons.Outlined.Verified, { onOpen("evidence") })
            }
        }
        item(key = "about") {
            SectionCard(title = "About", icon = Icons.Outlined.Info) {
                KeyValue("Version", versionName)
                KeyValue("Build", "TARGET · unsigned (PLAT-006)")
                HonestyLabel()
            }
        }
    }
}

// ----------------------------------------------------------------- activity

@Composable
fun ActivityContent(items: List<ActivityUi>) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(Space.l),
        verticalArrangement = Arrangement.spacedBy(Space.m),
    ) {
        if (items.isEmpty()) {
            item { EmptyState(Icons.Outlined.History, "No activity yet", "Sessions you prepare or receive appear here.") }
        } else {
            item {
                SectionCard(contentPadding = Space.s) {
                    items.forEachIndexed { i, a ->
                        if (i > 0) ListDivider()
                        ActivityListItem(a, a.role.icon())
                    }
                }
            }
        }
    }
}
