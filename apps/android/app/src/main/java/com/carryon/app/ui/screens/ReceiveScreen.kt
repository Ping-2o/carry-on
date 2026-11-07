/*
 * ReceiveScreen.kt — the «Принять» screen (stage 3, L0/L1 onboarding ladder,
 * manual-IP mode): arms a receiver on open (shows THIS device's address + full
 * pin for the source to dial/pin), takes the source's pin, a destination-
 * controlled chunk size (§18.6), then runs accept → import → CORE-004 gate →
 * ACTION_VIEW. The L0/L1 ladder + §16.2 session state + monospace engine
 * console + the DYNAMIC per-run disclosure (LOCAL vs PHYSICAL by descriptor).
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import com.carryon.app.demo.DemoRunner
import com.carryon.app.handoff.HandoffLadderStep
import com.carryon.app.handoff.HandoffOutcome
import com.carryon.app.handoff.HandoffValidation
import com.carryon.app.handoff.ReceiveRunner
import com.carryon.app.handoff.ReceiveState
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.components.KeyValueLine
import com.carryon.app.ui.components.MonoConsole
import com.carryon.app.ui.theme.ConsoleTextStyle
import com.carryon.app.ui.theme.AsciiTextStyle

@Composable
fun ReceiveScreen(runner: ReceiveRunner = AppGraph.receiveRunner) {
    val state by runner.state.collectAsState()
    val context = LocalContext.current
    var sourcePin by remember { mutableStateOf("") }
    var chunkSize by remember { mutableStateOf(0L) }
    val clipboard = LocalClipboardManager.current

    // Arm on first composition: the source needs THIS device's address + pin
    // before it serves (manual mode's honest human channel).
    LaunchedEffect(Unit) { runner.arm() }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Receive", style = MaterialTheme.typography.headlineSmall)
        Text(
            text = ReceiveState.RUN_LABEL,
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.tertiary,
        )

        // ---- this receiver's coordinates (shown to the source) ----
        Text("This device (give to the source)", style = MaterialTheme.typography.titleSmall)
        KeyValueLine("address", state.displayAddr.ifBlank { "arming…" })
        SelectionContainer {
            Text(
                text = "pin:  ${state.pinHex.ifBlank { "…" }}",
                style = ConsoleTextStyle,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { clipboard.setText(AnnotatedString(state.pinHex)) },
                enabled = state.pinHex.isNotBlank(),
            ) { Text("Copy full pin") }
            OutlinedButton(onClick = { runner.arm() }, enabled = !state.running) {
                Text("Re-arm")
            }
        }
        Text(
            "The pin is a certificate fingerprint (public, not a secret). The source pairs it,\nyou type its pin below — manual mode until discovery (stage 4).",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )

        // ---- the source's pin ----
        OutlinedTextField(
            value = sourcePin,
            onValueChange = { sourcePin = it.trim().lowercase() },
            label = { Text("Source (mac) pin — 64 hex") },
            modifier = Modifier.fillMaxWidth(),
            enabled = !state.running,
            singleLine = true,
            supportingText = {
                Text(
                    when {
                        sourcePin.isBlank() -> "read it off the source's console"
                        HandoffValidation.isValidPinHex(sourcePin) -> "64 hex ✓"
                        else -> "needs 64 lowercase hex — refusing (fail closed)"
                    },
                    style = ConsoleTextStyle,
                    color = if (HandoffValidation.isValidPinHex(sourcePin)) MaterialTheme.colorScheme.primary
                            else MaterialTheme.colorScheme.error,
                )
            },
        )

        // ---- destination-controlled chunk size (§18.6) ----
        Text("Chunk size (§18.6 — destination-controlled)", style = MaterialTheme.typography.titleSmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(
                selected = chunkSize == 0L,
                onClick = { chunkSize = 0L },
                label = { Text("default") },
                enabled = !state.running,
            )
            FilterChip(
                selected = chunkSize == 4096L,
                onClick = { chunkSize = 4096L },
                label = { Text("4 KiB") },
                enabled = !state.running,
            )
            FilterChip(
                selected = chunkSize == 65536L,
                onClick = { chunkSize = 65536L },
                label = { Text("64 KiB") },
                enabled = !state.running,
            )
        }

        Button(
            onClick = { runner.accept(sourcePin, chunkSize) },
            enabled = !state.running && state.armed && HandoffValidation.isValidPinHex(sourcePin),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (state.running) "RECEIVING…" else "▣ ACCEPT FILE")
        }

        state.error?.let {
            Text(
                text = "FAIL: $it",
                color = MaterialTheme.colorScheme.error,
                style = ConsoleTextStyle,
            )
        }

        // ---- the honest L0/L1 ladder ----
        Text("Continuity ladder", style = MaterialTheme.typography.titleSmall)
        HandoffLadderStep.entries.forEach { step ->
            val outcome = state.ladder[step] ?: HandoffOutcome.PENDING
            val color = when (outcome) {
                HandoffOutcome.PENDING -> MaterialTheme.colorScheme.outline
                HandoffOutcome.RUNNING -> MaterialTheme.colorScheme.primary
                HandoffOutcome.OK -> MaterialTheme.colorScheme.primary
                HandoffOutcome.SKIPPED -> MaterialTheme.colorScheme.secondary
                HandoffOutcome.FAILED -> MaterialTheme.colorScheme.error
            }
            val mark = when (outcome) {
                HandoffOutcome.PENDING -> "▫"
                HandoffOutcome.RUNNING -> "▶"
                HandoffOutcome.OK -> "▪"
                HandoffOutcome.SKIPPED -> "◐"
                HandoffOutcome.FAILED -> "✗"
            }
            val sub = when (step) {
                HandoffLadderStep.L0_CARRY -> "sha256 measured == declared (byte-for-byte, CORE-004)"
                HandoffLadderStep.L1_OPEN -> "engine document.open + OS ACTION_VIEW (§10.3 PLAT-003)"
            }
            Column {
                Text("$mark ${step.label}", style = AsciiTextStyle, color = color)
                Text(sub, style = ConsoleTextStyle, color = MaterialTheme.colorScheme.secondary)
            }
        }

        // ---- §16.2 session state + measured counters ----
        state.sessionState?.let { KeyValueLine("session_state", it) }
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            KeyValueLine("wire.sent", state.bytesSent.toString())
            KeyValueLine("wire.recv", state.bytesRecv.toString())
        }
        state.lastResult?.let { r ->
            KeyValueLine("file", "${r.descriptor.fileName} (${r.verifiedBytes?.length() ?: 0} B)")
            KeyValueLine("declared_sha256", r.declaredHash.take(16) + "…")
            r.resolvedActivity?.let { KeyValueLine("opened_in", it) }
            KeyValueLine("verify", "ok=${r.verifyOk} evid=${r.evidenceClass}")
        }

        // ---- engine console + DYNAMIC per-run disclosure footer ----
        if (state.console.isNotEmpty()) {
            MonoConsole(lines = state.console, footer = null)
        }
        Text(
            text = "Chunk size is destination-controlled (§18.6). Demo Mode stays the always-LOCAL showcase; this screen is the real handoff.",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        Spacer(Modifier.height(24.dp))
    }
}
