/*
 * SessionsScreen.kt — observed sessions with their EXACT §16.2 readiness states
 * (Idle, Preparing, CutProposed, CutSealed, Importing, ActionReady,
 * ReadOnlyActive, Failed, Inconclusive, Aborted) plus authority_epoch and
 * may_mutate. The bare word "Ready" is forbidden (§16.2) — and never shown.
 */
package com.carryon.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.ReadinessStates
import com.carryon.app.ui.components.AsciiBox
import com.carryon.app.ui.components.AsciiRule
import com.carryon.app.ui.components.KeyValueLine
import com.carryon.app.ui.theme.ConsoleTextStyle

@Composable
fun SessionsScreen() {
    val sessions by AppGraph.topology.sessions.collectAsState()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Sessions", style = MaterialTheme.typography.headlineSmall)
        Text(
            text = "Exact readiness states (spec §16.2): ${ReadinessStates.ALL.joinToString(" · ")}",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )

        if (sessions.isEmpty()) {
            Text(
                "no sessions yet — run the Demo",
                style = ConsoleTextStyle,
                color = MaterialTheme.colorScheme.secondary,
            )
        } else {
            sessions.forEach { s ->
                AsciiBox(
                    title = "SESSION",
                    lines = listOf(
                        "id:     ${s.sessionId}",
                        "title:  ${s.title}",
                        "state:  ${s.state}   (exact §16.2 state — never bare \"Ready\")",
                        "role:   ${s.role}",
                    ) + listOfNotNull(
                        s.authorityEpoch?.let { "epoch:  $it (authority)" },
                        s.mayMutate?.let { "may_mutate:  $it" },
                    ),
                )
                AsciiRule()
            }
        }
    }
}
