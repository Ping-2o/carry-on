/*
 * TopologyView.swift — live device topology (plan.md §3): local node card,
 * trust records with revoke, §16.2 session states, the ENGINE-side discovery
 * section (stage 4 — §18.3 TXT owned by the engine), and the ASCII device
 * graph with TLS links + counters.
 */

import CarryOnKit
import SwiftUI

struct TopologyView: View {
    @Environment(AppState.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                localNodeCard
                asciiGraph
                trustSection
                discoverySection
            }
            .padding(20)
            .frame(maxWidth: 900, alignment: .leading)
        }
        .navigationTitle("Topology")
    }

    // MARK: local node card

    private var localNodeCard: some View {
        GroupBox("Local node") {
            VStack(alignment: .leading, spacing: 6) {
                LabeledRow(key: "device", value: Host.current().localizedName ?? "mac")
                LabeledRow(key: "pin", value: app.topology.localPin.isEmpty
                           ? "generating…"
                           : String(app.topology.localPin.prefix(12)) + "…  (truncated)")
                LabeledRow(key: "abi", value: AbiVersion.current().description)
                LabeledRow(key: "listener",
                           value: app.demo.lastResult == nil ? "not listening"
                           : "per-run loopback listener (see demo console)")
                Text("identity key DER: SECRET (EVD-006/§22.3) — never displayed, logged, or persisted")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: ASCII device graph

    private var asciiGraph: some View {
        GroupBox("Device graph") {
            let r = app.demo.lastResult
            let sent = app.demo.bytesSent
            let recv = app.demo.bytesRecv
            let connected = r != nil
            Text(graphText(connected: connected))
                .font(Theme.mono)
                .foregroundStyle(connected ? Theme.wire : Theme.pending)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 16) {
                ByteCounter(label: "link sent", value: sent)
                ByteCounter(label: "link recv", value: recv)
                Spacer()
                Text(connected ? "═══ TLS 1.3 (loopback, LOCAL) ══" : "no run yet")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
            }
        }
    }

    private func graphText(connected: Bool) -> String {
        let link = connected ? "═══════ TLS ═══════" : "┄┄┄┄┄┄┄  idle  ┄┄┄┄┄┄┄"
        return """
        ┌────────────────────┐          ┌────────────────────┐
        │  THIS MAC (source) │ \(link) │  DEST (in-process) │
        │  carryon-core #1   │          │  carryon-core #2   │
        └────────────────────┘          └────────────────────┘
        """
    }

    // MARK: trust records

    private var trustSection: some View {
        GroupBox("Trust records (§18.4)") {
            VStack(alignment: .leading, spacing: 8) {
                if app.topology.records.isEmpty {
                    Text("no paired peers yet — run the demo to pair the two in-process cores")
                        .font(Theme.monoSmall).foregroundStyle(.secondary)
                } else {
                    ForEach(app.topology.records) { rec in
                        HStack(spacing: 12) {
                            Text(rec.device_name).font(Theme.mono)
                            Text(rec.truncatedPin).font(Theme.mono).foregroundStyle(.secondary)
                            Text("gen=\(rec.trust_generation)").font(Theme.monoSmall).foregroundStyle(.secondary)
                            Text("paired=\(rec.paired_utc)").font(Theme.monoSmall).foregroundStyle(.secondary)
                            if rec.revoked {
                                Text("REVOKED").font(Theme.monoSmall).foregroundStyle(Theme.failed)
                            } else {
                                Spacer()
                                Button("Revoke") { app.topology.revoke(rec) }
                                    .controlSize(.small)
                            }
                            Spacer(minLength: 0)
                        }
                        .textSelection(.enabled)
                    }
                }
                Text("trust JSON persists via carryon_trust_to_json / carryon_trust_from_json (pins are fingerprints, not secrets)")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: engine discovery (stage 4)

    private var discoverySection: some View {
        let d = app.discovery
        return GroupBox("Discovery (engine-side, §18.3)") {
            VStack(alignment: .leading, spacing: 8) {
                Text("The engine advertises and browses _carryon._tcp; the §18.3 minimal TXT (pm id pr dc ch) is\nconstructed and validated by the engine — foreign keys are ignored, malformed records are dropped.")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
                HStack(spacing: 16) {
                    Toggle("Advertise presence", isOn: Binding(
                        get: { d.advertising },
                        set: { on in
                            if on { d.startAdvertising() } else { d.stopAdvertising() }
                        }))
                    Toggle("Browse", isOn: Binding(
                        get: { d.browsing },
                        set: { on in
                            if on { d.startBrowsing() } else { d.stopBrowsing() }
                        }))
                    if d.advertising {
                        Text(d.advertisedName).font(Theme.monoSmall).foregroundStyle(.secondary)
                    }
                }
                if !d.lastError.isEmpty {
                    Text(d.lastError).font(Theme.monoSmall).foregroundStyle(Theme.skipped)
                }
                if d.peers.isEmpty {
                    Text(d.browsing
                         ? "no instances discovered (foreground-only; peers age out after the engine's TTL)"
                         : "browsing off")
                        .font(Theme.monoSmall)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(d.peers) { p in
                        Text("◦ \(p.instanceName)  pm=\(p.txt.protocol_major) pr=\(p.txt.pairing_required ? "true" : "false") dc=\(p.txt.device_class) ch=\(p.txt.connection_hint)  →  \(p.address)")
                            .font(Theme.mono)
                            .textSelection(.enabled)
                    }
                }
                if !d.rejections.isEmpty {
                    Text("filtered (fail-closed): \(d.rejections.joined(separator: " · "))")
                        .font(Theme.monoSmall)
                        .foregroundStyle(Theme.pending)
                        .textSelection(.enabled)
                }
                if let ms = d.firstPeerLatencyMs {
                    Text(String(format: "NET-003: first peer in %.0f ms (gate < 10 s)", ms))
                        .font(Theme.monoSmall)
                        .foregroundStyle(ms < 10_000 ? Theme.ok : Theme.failed)
                        .textSelection(.enabled)
                }
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Monospace `key:  value` row.
struct LabeledRow: View {
    let key: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Text(key + ":")
                .font(Theme.mono)
                .foregroundStyle(.secondary)
                .frame(width: 110, alignment: .leading)
            Text(value)
                .font(Theme.mono)
                .textSelection(.enabled)
            Spacer(minLength: 0)
        }
    }
}
