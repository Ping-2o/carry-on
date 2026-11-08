/*
 * ReceiveScreen.kt — the «Принять» screen (stage 6): arms a receiver on the
 * DURABLE Keystore identity (stable pin — the stage-5 pairing matches),
 * advertises it via NSD (engine §18.3 TXT), and runs the full chain
 * accept → import (suspend/resume legs §11.2) → CORE-004 gate → ACTION_VIEW.
 * A PAIRED source needs NO typing (unpaired/revoked fails the handshake
 * closed, §18.5); the manual typed-pin stays the fallback. §16.2 session
 * states, object-first progress, bytes_before_first_action in the UI,
 * monospace engine console + the DYNAMIC per-run disclosure.
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
import androidx.compose.runtime.DisposableEffect
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
    // The MANUAL FALLBACK: null (empty) = the PAIRED durable trust.
    var sourcePin by remember { mutableStateOf("") }
    var chunkSize by remember { mutableStateOf(0L) }
    var suspendDrill by remember { mutableStateOf(false) }
    var tinyBudget by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current

    // Arm on first composition: the durable identity + listener exist BEFORE
    // the source serves (the address/pin display stays as the manual channel).
    LaunchedEffect(Unit) { runner.arm() }
    val discovery by AppGraph.discovery.state.collectAsState()
    // Advertise the ARMED listener over NSD the moment its port exists: the
    // ENGINE builds the §18.3 TXT; the source's engine browse finds it.
    LaunchedEffect(state.displayAddr) {
        val port = state.displayAddr.substringAfterLast(':').toIntOrNull()
        if (state.armed && port != null && port > 0) {
            AppGraph.discovery.advertise(port = port)
        }
    }
    // Foreground-first (battery): stop advertising when the screen leaves.
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
        Text("Receive", style = MaterialTheme.typography.headlineSmall)
        Text(
            text = ReceiveState.RUN_LABEL,
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.tertiary,
        )

        // ---- this receiver's coordinates (the manual fallback channel) ----
        Text("This device (discovered via NSD; manual fallback below)", style = MaterialTheme.typography.titleSmall)
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
            "The pin is the DURABLE identity's certificate fingerprint (public, not a secret; stable\nacross launches — Keystore-wrapped keys, PLAT-002). This receiver is advertised via NSD\n(§18.3 engine TXT) so a paired source can discover + connect with NO typing.",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        KeyValueLine(
            "paired_sources",
            if (state.pairedRecords > 0) "${state.pairedRecords} (sealed store — accept needs no typing)"
            else "none — pair first (the Pairing tab) or use the manual pin below",
        )
        KeyValueLine(
            "nsd",
            if (discovery.advertising) "advertising ${discovery.advertisedName} (_carryon._tcp)"
            else if (discovery.lastError != null) "advertise: ${discovery.lastError}"
            else "advertise: idle (arming…)",
        )

        // ---- the source: PAIRED (no typing) or the manual typed pin ----
        Text(
            if (sourcePin.isBlank()) "Source: PAIRED trust (no typing)" else "Source: manual pin fallback",
            style = MaterialTheme.typography.titleSmall,
        )
        OutlinedTextField(
            value = sourcePin,
            onValueChange = { sourcePin = it.trim().lowercase() },
            label = { Text("Source pin — manual fallback (leave blank to use the PAIRED trust)") },
            modifier = Modifier.fillMaxWidth(),
            enabled = !state.running,
            singleLine = true,
            supportingText = {
                Text(
                    when {
                        sourcePin.isBlank() ->
                            if (state.pairedRecords > 0) "blank = the paired durable store pins the source (§18.5)"
                            else "no paired records — type the source's 64-hex pin (or pair in the Pairing tab)"
                        HandoffValidation.isValidPinHex(sourcePin) -> "64 hex ✓ (manual fallback)"
                        else -> "needs 64 lowercase hex — refusing (fail closed)"
                    },
                    style = ConsoleTextStyle,
                    color = if (sourcePin.isBlank() || HandoffValidation.isValidPinHex(sourcePin))
                        MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.error,
                )
            },
        )

        // ---- destination-controlled knobs (§18.6) + honest drills ----
        Text("Chunk size (§18.6 — destination-controlled) + drills", style = MaterialTheme.typography.titleSmall)
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
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(
                selected = suspendDrill,
                onClick = {
                    suspendDrill = !suspendDrill
                    runner.suspendDrill = suspendDrill
                },
                label = { Text("⏸ suspend mid-import (§11.2 drill)") },
                enabled = !state.running,
            )
            FilterChip(
                selected = tinyBudget,
                onClick = {
                    tinyBudget = !tinyBudget
                    runner.tinyBudget = tinyBudget
                },
                label = { Text("tiny budget (§6.8 drill)") },
                enabled = !state.running,
            )
        }
        Text(
            "The §11.2 drill checkpoints the import at a chunk boundary and resumes on a fresh connection —\nthe crash-scenario proof that the transfer is SAVED, not lost. The §6.8 drill refuses visibly.",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )

        Button(
            onClick = {
                // null = the PAIRED durable trust; non-null = manual fallback.
                runner.accept(sourcePin.ifBlank { null }, chunkSize)
            },
            enabled = !state.running && state.armed &&
                (sourcePin.isBlank() && state.pairedRecords > 0 || HandoffValidation.isValidPinHex(sourcePin)),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (state.running) "RECEIVING…" else "▣ ACCEPT FROM SOURCE")
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
                HandoffLadderStep.L0_CARRY -> "sha256 measured == declared (byte-for-byte, CORE-004) — 1/1 object"
                HandoffLadderStep.L1_OPEN -> "engine document.open + OS ACTION_VIEW (§10.3 PLAT-003)"
            }
            Column {
                Text("$mark ${step.label}", style = AsciiTextStyle, color = color)
                Text(sub, style = ConsoleTextStyle, color = MaterialTheme.colorScheme.secondary)
            }
        }

        // ---- §16.2 session state + measured counters (objects first) ----
        state.sessionState?.let { KeyValueLine("session_state", it) }
        state.legsUsed.let { legs ->
            if (legs > 0) KeyValueLine("import_legs", "$legs${if (legs > 1) " (a cooperative suspend was resumed, §11.2)" else ""}")
        }
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            KeyValueLine("wire.sent", state.bytesSent.toString())
            KeyValueLine("wire.recv", state.bytesRecv.toString())
        }
        state.bytesBeforeFirstAction.let {
            if (it > 0) KeyValueLine("bytes_before_first_action", it.toString())
        }
        state.lastResult?.let { r ->
            KeyValueLine("file", "${r.descriptor.fileName} (${r.verifiedBytes?.length() ?: 0} B)")
            KeyValueLine("declared_sha256", r.declaredHash.take(16) + "…")
            r.resolvedActivity?.let { KeyValueLine("opened_in", it) }
            KeyValueLine("verify", "ok=${r.verifyOk} evid=${r.evidenceClass} trust=${if (r.pairedTrust) "paired" else "typed"}")
        }

        // ---- engine console + DYNAMIC per-run disclosure footer ----
        if (state.console.isNotEmpty()) {
            MonoConsole(lines = state.console, footer = null)
        }
        Text(
            text = "Chunk/budget are this destination's knobs (§18.6/§6.8). Progress is by OBJECTS (1/1),\nnot bytes; wire counters are the honest secondary measurement (§5.6). Demo Mode stays\nthe always-LOCAL showcase; this screen is the real cross-device receive.",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        Spacer(Modifier.height(24.dp))
    }
}
