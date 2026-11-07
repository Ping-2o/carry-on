/*
 * DemoScreen.kt — Demo Mode UI (plan.md §2): scenario picker, advanced toggles,
 * the animated 8-stage ASCII pipeline, rolling byte counters, may_mutate flips,
 * the monospace engine console, and the verbatim DISCLOSURE footer.
 *
 * Label is law (§2): the run is OBSERVED (LOCAL) — one device, loopback TLS 1.3.
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
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.carryon.app.demo.DemoRunner
import com.carryon.app.demo.DemoState
import com.carryon.app.demo.Scenario
import com.carryon.app.demo.Stage
import com.carryon.app.demo.StageStatus
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.components.ByteBlocks
import com.carryon.app.ui.components.KeyValueLine
import com.carryon.app.ui.components.MonoConsole
import com.carryon.app.ui.components.StagePipeline
import com.carryon.app.ui.theme.AsciiTextStyle
import com.carryon.app.ui.theme.ConsoleTextStyle

@Composable
fun DemoScreen(runner: DemoRunner = AppGraph.demoRunner) {
    val state by runner.state.collectAsState()

    // Record the completed run into the topology store (pairing + sessions).
    LaunchedEffect(state.running, state.evidenceBundleJson) {
        if (!state.running && state.evidenceBundleJson != null) {
            AppGraph.topology.recordDemoPairing(
                state.sourcePinHex, state.destPinHex,
                java.text.SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", java.util.Locale.US)
                    .format(java.util.Date()),
            )
            AppGraph.topology.recordDemoSession(state)
            AppGraph.topology.setListenerAddr(null) // wire closed with the run
        }
    }
    LaunchedEffect(state.listenerAddr) {
        if (state.listenerAddr.isNotBlank()) {
            AppGraph.topology.setListenerAddr(state.listenerAddr)
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Demo Mode", style = MaterialTheme.typography.headlineSmall)
        Text(
            text = DemoState.RUN_LABEL,
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.tertiary,
        )

        // ---- scenario picker ----
        Text("Scenario", style = MaterialTheme.typography.titleSmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Scenario.entries.forEach { scenario ->
                FilterChip(
                    selected = state.scenario == scenario,
                    onClick = { runner.setScenario(scenario) },
                    label = { Text(scenario.label) },
                    enabled = !state.running,
                )
            }
        }

        // ---- advanced showcase toggles ----
        Text("Advanced", style = MaterialTheme.typography.titleSmall)
        ToggleRow(
            title = "Chunk size 4 KiB",
            subtitle = "progressive transfer (core_set_chunk_size, §18.6; engine admits [4096 B, 8 MiB])",
            checked = state.toggles.chunkSize > 0,
            enabled = !state.running,
            onChange = {
                runner.setToggles(
                    // 4096 = the engine's minimum admitted chunk (§18.6); the
                    // runner clamps whatever lands here into [4096, 8388608].
                    state.toggles.copy(chunkSize = if (it) 4096L else 0L)
                )
            },
        )
        ToggleRow(
            title = "Budget rejection",
            subtitle = "tiny net budget → refusal/partial — a NORMAL outcome (§6.8)",
            checked = state.toggles.budgetRejection,
            enabled = !state.running,
            onChange = {
                runner.setToggles(state.toggles.copy(budgetRejection = it))
            },
        )
        ToggleRow(
            title = "Suspend / resume",
            subtitle = "request_suspend mid-import → resume on fresh connection (§11.2)",
            checked = state.toggles.suspendResume,
            enabled = !state.running,
            onChange = {
                runner.setToggles(state.toggles.copy(suspendResume = it))
            },
        )
        ToggleRow(
            title = "Recovery report",
            subtitle = "recovery_report_json fields (interrupted ≠ success, §19.3)",
            checked = state.toggles.recoveryReport,
            enabled = !state.running,
            onChange = {
                runner.setToggles(state.toggles.copy(recoveryReport = it))
            },
        )

        Button(
            onClick = { runner.runDemo() },
            enabled = !state.running,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (state.running) "RUNNING…" else "▶ RUN DEMO")
        }

        state.error?.let {
            Text(
                text = "FAIL: $it",
                color = MaterialTheme.colorScheme.error,
                style = ConsoleTextStyle,
            )
        }

        // ---- animated ASCII pipeline (8 stages) ----
        val runningIdx = state.stages.indexOfFirst { it.status == StageStatus.RUNNING }
        val doneThrough = state.stages.count { it.status == StageStatus.DONE }
        StagePipeline(
            stages = Stage.entries.map { it.label },
            runningIndex = runningIdx,
            doneThrough = if (runningIdx >= 0) doneThrough else doneThrough,
        )
        ByteBlocks(done = doneThrough, total = Stage.entries.size)

        // ---- rolling byte counters ----
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            KeyValueLine("wire.sent", formatBytes(state.bytesSent))
            KeyValueLine("wire.recv", formatBytes(state.bytesRecv))
        }

        // ---- stage states ----
        state.stages.forEach { stageUi ->
            val color = when (stageUi.status) {
                StageStatus.PENDING -> MaterialTheme.colorScheme.outline
                StageStatus.RUNNING -> MaterialTheme.colorScheme.primary
                StageStatus.DONE -> MaterialTheme.colorScheme.onSurface
                StageStatus.FAILED -> MaterialTheme.colorScheme.error
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "${stageUi.stage.label}",
                    style = AsciiTextStyle,
                    fontWeight = if (stageUi.status == StageStatus.RUNNING) FontWeight.Bold else FontWeight.Normal,
                    color = color,
                )
                Text(
                    text = stageUi.detail.ifBlank { stageUi.status.name.lowercase() },
                    style = ConsoleTextStyle,
                    color = color,
                )
            }
        }

        // ---- may_mutate flip (single-writer proof) ----
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            KeyValueLine(
                "may_mutate.src",
                state.sourceMayMutate?.toString() ?: "—",
            )
            KeyValueLine(
                "may_mutate.dst",
                state.destMayMutate?.toString() ?: "—",
            )
        }
        state.authorityEpoch?.let { KeyValueLine("authority_epoch", it.toString()) }
        state.listenerAddr.takeIf { it.isNotBlank() }?.let { KeyValueLine("listener", it) }

        // ---- engine console + verbatim disclosure footer ----
        if (state.console.isNotEmpty()) {
            MonoConsole(lines = state.console, footer = DemoState.DISCLOSURE)
        } else {
            Text(
                text = DemoState.DISCLOSURE,
                style = ConsoleTextStyle,
                color = MaterialTheme.colorScheme.tertiary,
            )
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun ToggleRow(
    title: String,
    subtitle: String,
    checked: Boolean,
    enabled: Boolean,
    onChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(
                subtitle,
                style = ConsoleTextStyle,
                color = MaterialTheme.colorScheme.secondary,
            )
        }
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

private fun formatBytes(bytes: Long): String = when {
    bytes >= 1_000_000 -> String.format(java.util.Locale.US, "%.1f MB", bytes / 1e6)
    bytes >= 1_000 -> String.format(java.util.Locale.US, "%.1f kB", bytes / 1e3)
    else -> "$bytes B"
}
