/*
 * TopologyScreen.kt — Live Device Topology (plan.md §3): local node card,
 * trust records + revoke, sessions with exact §16.2 states, the discovery
 * pilot section, and the ASCII device graph.
 */
package com.carryon.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.carryon.app.discovery.DISCOVERY_LABEL
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.ReadinessStates
import com.carryon.app.ui.components.AsciiBox
import com.carryon.app.ui.components.AsciiRule
import com.carryon.app.ui.components.KeyValueLine
import com.carryon.app.ui.theme.ConsoleTextStyle

@Composable
fun TopologyScreen() {
    val topology = AppGraph.topology
    val identity by topology.identity.collectAsState()
    val trustRecords by topology.trust.collectAsState()
    val sessions by topology.sessions.collectAsState()
    val discovery by AppGraph.discovery.state.collectAsState()

    LaunchedEffect(Unit) {
        topology.ensureLoaded()
        AppGraph.discovery.advertise(port = 0)
        AppGraph.discovery.startBrowse()
    }
    // Tear down NSD when the screen leaves composition (tab switch / exit).
    DisposableEffect(Unit) {
        onDispose { AppGraph.discovery.stop() }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Device Topology", style = MaterialTheme.typography.headlineSmall)

        // ---- local node card ----
        val id = identity
        if (id == null) {
            Text("opening engine identity…", style = ConsoleTextStyle)
        } else {
            AsciiBox(
                title = "LOCAL NODE",
                lines = listOf(
                    "name:  ${id.deviceName}",
                    "pin:   ${id.shortPin}  (SHA-256, truncated — §22.3)",
                    "abi:   engine ${id.abi.major}.${id.abi.minor}",
                    "listen: ${id.listenerAddr ?: "not listening"}",
                ),
            )
        }
        Text(
            text = "Identity is per-app-start in this pilot; durable secure storage is PLAT-002 (TARGET).",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )

        AsciiRule()

        // ---- ASCII device graph ----
        Text("Graph", style = MaterialTheme.typography.titleSmall)
        AsciiBox(
            title = "DEVICES",
            lines = buildList {
                val thisDevice = identity?.deviceName ?: "this-device"
                add("┌─────────────────┐")
                add("│ $thisDevice".padEnd(17) + "│")
                add("└─────────────────┘")
                val peers = discovery.peers
                if (peers.isEmpty()) {
                    add("      · (no _carryon._tcp peers visible)")
                } else {
                    peers.forEach { p ->
                        add("  ═══ TLS ══ ${p.serviceName}  ${p.deviceClass}")
                    }
                }
            },
        )

        AsciiRule()

        // ---- trust records ----
        Text("Trust records (§18.4)", style = MaterialTheme.typography.titleSmall)
        if (trustRecords.isEmpty()) {
            Text("none — run the Demo to pair", style = ConsoleTextStyle)
        } else {
            trustRecords.forEach { rec ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Column {
                        KeyValueLine("device", rec.deviceName)
                        KeyValueLine("pin", rec.shortPin)
                        KeyValueLine(
                            "state",
                            "gen=${rec.trustGeneration} revoked=${rec.revoked} paired=${rec.pairedUtc}",
                        )
                    }
                    if (!rec.revoked) {
                        TextButton(onClick = { topology.revoke(rec.pinHex) }) {
                            Text("revoke")
                        }
                    }
                }
                AsciiRule()
            }
        }

        AsciiRule()

        // ---- sessions with exact §16.2 states ----
        Text("Sessions", style = MaterialTheme.typography.titleSmall)
        Text(
            text = "readiness states: ${ReadinessStates.ALL.joinToString(", ")}",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        if (sessions.isEmpty()) {
            Text("no sessions yet — run the Demo", style = ConsoleTextStyle)
        } else {
            sessions.forEach { s ->
                KeyValueLine("session", "${s.sessionId.take(18)}… ${s.state}")
                KeyValueLine("role", s.role)
                s.authorityEpoch?.let { KeyValueLine("authority_epoch", it.toString()) }
                s.mayMutate?.let { KeyValueLine("may_mutate", it.toString()) }
                AsciiRule()
            }
        }

        AsciiRule()

        // ---- engine-contract discovery (§18.3) ----
        Text("Discovery", style = MaterialTheme.typography.titleSmall)
        Text(
            text = DISCOVERY_LABEL,
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.tertiary,
        )
        KeyValueLine(
            "advertise",
            if (discovery.advertising) "${discovery.advertisedName} (_carryon._tcp)" else "not advertising",
        )
        KeyValueLine("browse", if (discovery.browsing) "browsing _carryon._tcp" else "idle")
        discovery.firstPeerLatencyMs?.let { ms ->
            KeyValueLine(
                "NET-003",
                "first peer in $ms ms (gate < 10 s) — ${if (ms < 10_000) "PASS" else "FAIL"}",
            )
        }
        discovery.peers.forEach { p ->
            KeyValueLine(
                "peer",
                "${p.serviceName} dc=${p.deviceClass} pm=${p.protocolMajor} " +
                    "pr=${if (p.pairingRequired) "1" else "0"} → ${p.host}:${p.port}",
            )
            if (p.ignoredKeys.isNotEmpty()) {
                KeyValueLine("ignored_keys", p.ignoredKeys.joinToString(","))
            }
        }
        discovery.rejections.forEach { why ->
            KeyValueLine("filtered", why)
        }
        discovery.lastError?.let {
            Text("nsd:  $it", style = ConsoleTextStyle, color = MaterialTheme.colorScheme.error)
        }
        Text(
            text = "TXT keys are the §18.3 minimal set only (pm, id, pr, dc, ch) — built and validated by the ENGINE;\nforeign keys are ignored, malformed records are dropped (fail closed). Never user/session/app names or keys.",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        Spacer(Modifier.height(24.dp))
    }
}
