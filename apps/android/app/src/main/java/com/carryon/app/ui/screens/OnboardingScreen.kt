/*
 * OnboardingScreen.kt — 3 steps, then lands on the home screen (plan.md §1):
 *   1 "Begin here → continue there" — typewriter CARRY-ON ASCII banner.
 *   2 "What carries" — the L0–L4 ladder as ASCII.
 *   3 "The lifecycle" — animated ASCII pipeline + the honest note
 *     (single-writer, two-phase publish, interrupted ≠ success).
 * Reduced motion (animator scale 0) → all reveals are instant.
 */
package com.carryon.app.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.carryon.app.ui.LocalReducedMotion
import com.carryon.app.ui.components.AsciiBox
import com.carryon.app.ui.components.ByteBlocks
import com.carryon.app.ui.components.LifecyclePipeline
import com.carryon.app.ui.components.TypewriterText
import com.carryon.app.ui.components.carryOnBanner
import com.carryon.app.ui.components.l4LadderLines
import com.carryon.app.ui.theme.AsciiTextStyle

private const val STEP_COUNT = 3

@Composable
fun OnboardingScreen(onDone: () -> Unit) {
    val reducedMotion = LocalReducedMotion.current
    var step by remember { mutableIntStateOf(1) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        when (step) {
            1 -> {
                TypewriterText(
                    text = carryOnBanner(),
                    reducedMotion = reducedMotion,
                )
                Spacer(Modifier.height(12.dp))
                Text(
                    text = "Begin here → continue there",
                    style = MaterialTheme.typography.headlineSmall,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "Begin supported work on one device; continue the\n" +
                        "declared action on another. Typed continuation —\n" +
                        "never process or memory migration.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.secondary,
                )
            }
            2 -> {
                Text(
                    text = "What carries",
                    style = MaterialTheme.typography.headlineSmall,
                )
                Spacer(Modifier.height(12.dp))
                AsciiBox(title = "LEVELS", lines = l4LadderLines())
                Spacer(Modifier.height(8.dp))
                ByteBlocks(done = 4, total = 5)
            }
            else -> {
                Text(
                    text = "The lifecycle",
                    style = MaterialTheme.typography.headlineSmall,
                )
                Spacer(Modifier.height(12.dp))
                LifecyclePipeline()
                Spacer(Modifier.height(10.dp))
                Text(
                    text = "single-writer · two-phase publish\ninterrupted ≠ success",
                    style = AsciiTextStyle,
                    color = MaterialTheme.colorScheme.tertiary,
                    textAlign = TextAlign.Center,
                )
            }
        }

        Spacer(Modifier.height(28.dp))
        ByteBlocks(done = step, total = STEP_COUNT)
        Spacer(Modifier.height(8.dp))
        if (step < STEP_COUNT) {
            Button(onClick = { step += 1 }, modifier = Modifier.fillMaxWidth()) {
                Text("Next")
            }
        } else {
            Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) {
                Text("Start")
            }
        }
        TextButton(onClick = onDone) {
            Text("skip")
        }
    }
}
