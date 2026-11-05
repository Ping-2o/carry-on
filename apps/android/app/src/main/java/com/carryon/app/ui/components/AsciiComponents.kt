/*
 * AsciiComponents.kt — the repo's ASCII style as Compose primitives:
 * box-drawing frames (`─ │ ┌ ┐ └ ┘ ═ ▶`), byte blocks (`▪▪▪▫▫▫`), monospace
 * `key:  value` console lines, and a typewriter reveal that HONORS reduced
 * motion (Android animator duration scale 0 → full text immediately).
 */
package com.carryon.app.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.carryon.app.ui.theme.AsciiTextStyle
import com.carryon.app.ui.theme.BannerTextStyle
import com.carryon.app.ui.theme.ConsoleTextStyle
import com.carryon.app.ui.theme.Mono

/** Byte-block progress row: `done` filled ▪, the rest ▫. E.g. `▪▪▪▫▫▫`. */
@Composable
fun ByteBlocks(done: Int, total: Int, modifier: Modifier = Modifier) {
    Text(
        text = "▪".repeat(done.coerceIn(0, total)) + "▫".repeat((total - done).coerceAtLeast(0)),
        style = AsciiTextStyle,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier,
    )
}

/**
 * The 8-stage demo pipeline as an ASCII rail. Each stage is a box that lights up
 * (filled ▪) when running/done. Reduced motion → no per-stage animation delay.
 */
@Composable
fun StagePipeline(
    stages: List<String>,
    runningIndex: Int,
    doneThrough: Int,
    modifier: Modifier = Modifier,
) {
    val marks = stages.mapIndexed { i, label ->
        val mark = when {
            i < doneThrough -> "▪"
            i == doneThrough && runningIndex < 0 -> "▪"
            else -> if (i == runningIndex) "▶" else "▫"
        }
        "$mark $label"
    }
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            text = "┌─ PIPELINE " + "─".repeat(18) + "┐",
            style = AsciiTextStyle,
            color = MaterialTheme.colorScheme.outline,
        )
        marks.forEach { line ->
            Text(
                text = "│ $line",
                style = AsciiTextStyle,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
        Text(
            text = "└" + "─".repeat(29) + "┘",
            style = AsciiTextStyle,
            color = MaterialTheme.colorScheme.outline,
        )
    }
}

/** The lifecycle banner (onboarding step 3): CREATE CUT ─▶ SEAL ─▶ … verbatim. */
@Composable
fun LifecyclePipeline(modifier: Modifier = Modifier) {
    Text(
        text = "CREATE CUT ─▶ SEAL ─▶ TLS 1.3 ─▶ VERIFY ─▶ PUBLISH ─▶ ACTION READY",
        style = AsciiTextStyle,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier,
        textAlign = TextAlign.Center,
    )
}

/** Typewriter reveal: shows `text` character by character; when reducedMotion,
 *  shows the full text at once (plan.md: honor ANIMATOR_DURATION_SCALE == 0). */
@Composable
fun TypewriterText(
    text: String,
    reducedMotion: Boolean,
    style: androidx.compose.ui.text.TextStyle = BannerTextStyle,
    modifier: Modifier = Modifier,
) {
    var shown by remember(text) { mutableIntStateOf(if (reducedMotion) text.length else 0) }
    LaunchedEffect(text, reducedMotion) {
        if (reducedMotion) {
            shown = text.length
        } else {
            while (shown < text.length) {
                shown += 1
                kotlinx.coroutines.delay(18)
            }
        }
    }
    Text(
        text = text.take(shown),
        style = style,
        color = MaterialTheme.colorScheme.onSurface,
        modifier = modifier,
    )
}

/** The CARRY-ON box-drawing logo banner (onboarding step 1). */
fun carryOnBanner(): String = buildString {
    appendLine("┌──────────────────────────────┐")
    appendLine("│  ╔════════════════════════╗  │")
    appendLine("│  ║  C A R R Y - O N       ║  │")
    appendLine("│  ╚════════════════════════╝  │")
    appendLine("└──────────────────────────────┘")
}

/** Monospace engine console panel (`key:  value` lines, repo style). */
@Composable
fun MonoConsole(
    lines: List<String>,
    modifier: Modifier = Modifier,
    footer: String? = null,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.small,
    ) {
        Column(Modifier.padding(8.dp)) {
            lines.forEach { line ->
                Text(
                    text = line,
                    style = ConsoleTextStyle,
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
            footer?.let {
                Text(
                    text = it,
                    style = ConsoleTextStyle,
                    color = MaterialTheme.colorScheme.tertiary,
                    modifier = Modifier.padding(top = 6.dp),
                )
            }
        }
    }
}

/** Box-drawing frame around `title` + content lines (topology graph, ladder). */
@Composable
fun AsciiBox(
    title: String,
    lines: List<String>,
    modifier: Modifier = Modifier,
    reducedMotion: Boolean = true,
    typewriter: Boolean = false,
) {
    Column(modifier = modifier) {
        val width = maxOf(title.length + 4, (lines.maxOfOrNull { it.length } ?: 0) + 3)
        Text(
            text = "┌─ $title " + "─".repeat((width - title.length - 4).coerceAtLeast(0)) + "┐",
            style = AsciiTextStyle,
            color = MaterialTheme.colorScheme.outline,
        )
        if (typewriter && !reducedMotion) {
            lines.forEach { line ->
                TypewriterText(
                    text = "│ $line",
                    reducedMotion = false,
                    style = AsciiTextStyle,
                )
            }
        } else {
            lines.forEach { line ->
                Text(
                    text = "│ $line",
                    style = AsciiTextStyle,
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
        Text(
            text = "└" + "─".repeat(width) + "┘",
            style = AsciiTextStyle,
            color = MaterialTheme.colorScheme.outline,
        )
    }
}

/** A `key:  value` monospace line. */
@Composable
fun KeyValueLine(key: String, value: String, modifier: Modifier = Modifier) {
    Row(modifier = modifier) {
        Text(
            text = key.padEnd(18) + " ",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
            fontFamily = Mono,
        )
        Text(text = value, style = ConsoleTextStyle, color = MaterialTheme.colorScheme.onSurface)
    }
}

/** Dimmed horizontal rule in box style. */
@Composable
fun AsciiRule(modifier: Modifier = Modifier) {
    Text(
        text = "─".repeat(46),
        style = AsciiTextStyle,
        color = MaterialTheme.colorScheme.outline.copy(alpha = 0.5f),
        modifier = modifier,
    )
}

/** The honest L0–L4 ladder (onboarding step 2) — verbatim from plan.md. */
fun l4LadderLines(): List<String> = listOf(
    "L4 ─ single-writer authority transfer",
    "L3 ─ structured session (document + unsaved + meta)",
    "L2 ─ typed objects + actions",
    "L1 ─ launcher activation",
    "L0 ─ files",
)
