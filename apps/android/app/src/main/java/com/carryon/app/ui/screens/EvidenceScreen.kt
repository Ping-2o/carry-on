/*
 * EvidenceScreen.kt — parses the EvidenceBundle JSON from the last demo run
 * (plan.md §4): bundle_version, session_id, created_utc, verifier_version,
 * build{crate_version,schema_version,phase}, session_json, journal[], actions[],
 * metrics{}, section_hashes[[name,sha256]], disclosure — shown verbatim.
 * No bundle content is ever executed (EVD-005); this screen only renders text.
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
import com.carryon.app.ui.components.AsciiBox
import com.carryon.app.ui.components.AsciiRule
import com.carryon.app.ui.components.KeyValueLine
import com.carryon.app.ui.components.MonoConsole
import com.carryon.app.ui.theme.ConsoleTextStyle
import org.json.JSONObject

@Composable
fun EvidenceScreen() {
    val demo by AppGraph.demoRunner.state.collectAsState()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text("Evidence", style = MaterialTheme.typography.headlineSmall)
        Text(
            text = "Rendered only — bundle content is never executed (EVD-005).",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )

        val bundleJson = demo.evidenceBundleJson
        if (bundleJson == null) {
            Text(
                "no bundle yet — run the Demo",
                style = ConsoleTextStyle,
                color = MaterialTheme.colorScheme.secondary,
            )
            MonoConsole(lines = emptyList(), footer = null)
            return@Column
        }

        val report = demo.verifyReportJson
        val bundle = try {
            JSONObject(bundleJson)
        } catch (_: Exception) {
            null
        }
        if (bundle == null) {
            Text("bundle parse error", color = MaterialTheme.colorScheme.error)
            return@Column
        }

        // ---- header ----
        AsciiBox(
            title = "BUNDLE",
            lines = listOf(
                "bundle_version:     ${bundle.optInt("bundle_version")}",
                "session_id:         ${bundle.optString("session_id")}",
                "created_utc:        ${bundle.optString("created_utc")}",
                "verifier_version:   ${bundle.optString("verifier_version")}",
            ),
        )

        // ---- build ----
        bundle.optJSONObject("build")?.let { build ->
            Text("Build", style = MaterialTheme.typography.titleSmall)
            KeyValueLine("crate_version", build.optString("crate_version"))
            KeyValueLine("schema_version", build.optInt("schema_version").toString())
            val phase = build.optString("phase")
            if (phase.isNotBlank()) KeyValueLine("phase", phase)
        }

        AsciiRule()

        // ---- section hashes ----
        Text("Section hashes", style = MaterialTheme.typography.titleSmall)
        bundle.optJSONArray("section_hashes")?.let { hashes ->
            for (i in 0 until hashes.length()) {
                val pair = hashes.optJSONArray(i) ?: continue
                val name = pair.optString(0)
                val sha = pair.optString(1)
                // Truncated display; the bundle carries the full hash.
                KeyValueLine(name, sha.take(24) + if (sha.length > 24) "…" else "")
            }
        }

        AsciiRule()

        // ---- journal timeline ----
        Text("Journal", style = MaterialTheme.typography.titleSmall)
        bundle.optJSONArray("journal")?.let { journal ->
            val lines = buildList {
                for (i in 0 until journal.length()) {
                    val row = journal.optJSONObject(i) ?: continue
                    val ts = row.optString("ts_utc", row.optString("ts", "?"))
                    val kind = row.optString("kind", row.optString("event", "?"))
                    add("${ts.take(19)}  $kind")
                }
            }
            if (lines.isEmpty()) {
                Text("(empty journal)", style = ConsoleTextStyle)
            } else {
                AsciiBox(title = "TIMELINE", lines = lines.take(40))
            }
        }

        AsciiRule()

        // ---- metrics ----
        Text("Metrics", style = MaterialTheme.typography.titleSmall)
        bundle.optJSONObject("metrics")?.let { metrics ->
            MonoConsole(
                lines = metrics.keys().asSequence().sorted().map { key ->
                    "$key:  ${metrics.opt(key)}"
                }.toList(),
            )
        }

        AsciiRule()

        // ---- verification report ----
        if (report != null) {
            Text("Verification", style = MaterialTheme.typography.titleSmall)
            MonoConsole(lines = listOf(report.take(400)))
        }

        AsciiRule()

        // ---- disclosure verbatim (§23.3) ----
        Text("Disclosure", style = MaterialTheme.typography.titleSmall)
        Text(
            text = bundle.optString("disclosure"),
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.tertiary,
        )
    }
}
