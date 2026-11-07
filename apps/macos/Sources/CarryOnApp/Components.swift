/*
 * Components.swift — the reusable ASCII/SwiftUI parts: typewriter banner,
 * lifecycle pipeline diagram, byte blocks + rolling counters, and the engine
 * console with the fixed DISCLOSURE footer (plan.md wording, verbatim).
 */

import CarryOnKit
import SwiftUI

/// Typewriter reveal of an ASCII banner; honors Reduce Motion (full text at once).
struct AsciiBanner: View {
    let text: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = 0

    var body: some View {
        Text(String(text.prefix(effectiveCount)))
            .font(Theme.monoTitle)
            .foregroundStyle(Theme.accent)
            .textSelection(.enabled)
            .task {
                guard !reduceMotion else {
                    shown = text.count
                    return
                }
                shown = 0
                for _ in 0..<text.count {
                    try? await Task.sleep(nanoseconds: 55_000_000)
                    shown += 1
                }
            }
    }

    private var effectiveCount: Int {
        reduceMotion ? text.count : max(shown, 0)
    }
}

/// The onboarding lifecycle diagram (plan.md §1 step 3, exact arrows).
struct LifecycleDiagram: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var lit = 0

    private let steps = ["CREATE CUT", "SEAL", "TLS 1.3", "VERIFY", "PUBLISH", "ACTION READY"]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                ForEach(steps.indices, id: \.self) { i in
                    Text(steps[i])
                        .font(Theme.mono)
                        .foregroundStyle(i < lit ? Theme.ok : Theme.pending)
                    if i < steps.count - 1 {
                        Text("─▶").font(Theme.mono).foregroundStyle(Theme.pending)
                    }
                }
            }
            .textSelection(.enabled)
            VStack(alignment: .leading, spacing: 2) {
                Text("single-writer: one owner per session (§21.2)").font(Theme.monoSmall).foregroundStyle(.secondary)
                Text("two-phase publish: stage → verify → atomic (CORE-004)").font(Theme.monoSmall).foregroundStyle(.secondary)
                Text("interrupted ≠ success: partials discarded, failures stay in the record (§19.3.9)").font(Theme.monoSmall).foregroundStyle(.secondary)
            }
        }
        .task { await animate() }
    }

    private func animate() async {
        lit = reduceMotion ? steps.count : 0
        while lit < steps.count {
            try? await Task.sleep(nanoseconds: reduceMotion ? 1 : 320_000_000)
            lit += 1
        }
    }
}

/// Byte blocks `▪▪▪▫▫▫` — fill reflects received bytes (log-scaled, 6 blocks).
struct ByteBlocks: View {
    let recv: UInt64
    let active: Bool

    private var filled: Int {
        guard recv > 0 else { return 0 }
        return min(6, max(1, Int(log2(Double(recv + 1)))))
    }

    var body: some View {
        let s = String(repeating: "▪", count: filled) + String(repeating: "▫", count: 6 - filled)
        Text(s)
            .font(Theme.mono)
            .foregroundStyle(active ? Theme.wire : Theme.pending)
            .monospacedDigit()
            .textSelection(.enabled)
            .accessibilityLabel("bytes received, \(filled) of 6 blocks")
    }
}

/// Rolling byte counters (`contentTransition(.numericText)`; reduce-motion skips).
struct ByteCounter: View {
    let label: String
    let value: UInt64
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 4) {
            Text("\(label):").font(Theme.mono).foregroundStyle(.secondary)
            Text("\(value)")
                .font(Theme.mono)
                .monospacedDigit()
                .contentTransition(reduceMotion ? .identity : .numericText())
                .textSelection(.enabled)
        }
    }
}

/// One stage cell in the 8-stage pipeline row: `▪ LABEL` lit, `▫ LABEL` pending.
struct StageCell: View {
    let stage: DemoStage
    let outcome: StageOutcome?
    let running: Bool

    private var glyph: String {
        if running { return "▪" }
        switch outcome {
        case .ok: return "▪"
        case .skipped: return "◐"
        case .failed: return "✗"
        case nil: return "▫"
        }
    }

    private var color: Color {
        if running { return Theme.accent }
        switch outcome {
        case .ok: return Theme.ok
        case .skipped: return Theme.skipped
        case .failed: return Theme.failed
        case nil: return Theme.pending
        }
    }

    var body: some View {
        HStack(spacing: 3) {
            Text(glyph).font(Theme.mono).foregroundStyle(color)
            Text(stage.label).font(Theme.monoSmall).foregroundStyle(outcome == nil && !running ? Theme.pending : .primary)
        }
    }
}

/// The full 8-stage ASCII pipeline row with ─▶ connectors.
struct StagePipeline: View {
    let states: [DemoStage: StageOutcome]
    let runningStage: DemoStage?

    var body: some View {
        HStack(spacing: 6) {
            ForEach(DemoStage.allCases) { stage in
                StageCell(stage: stage,
                          outcome: states[stage],
                          running: runningStage == stage)
                if stage.rawValue < DemoStage.allCases.count {
                    Text("─▶").font(Theme.monoSmall).foregroundStyle(Theme.pending)
                }
            }
        }
        .textSelection(.enabled)
    }
}

/// The engine console: monospace `key:  value` lines, auto-scroll, and the
/// fixed DISCLOSURE footer (plan.md wording, verbatim) once a run exists.
struct EngineConsole: View {
    let lines: [ConsoleLine]
    let showFooter: Bool

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    ForEach(lines) { line in
                        Text(line.consoleText)
                            .font(Theme.mono)
                            .foregroundStyle(line.key.isEmpty && !line.value.isEmpty
                                             && line.value.hasPrefix("DISCLOSURE") ? .secondary : .primary)
                            .textSelection(.enabled)
                            .id(line.id)
                    }
                    if showFooter {
                        ForEach(DemoResult.disclosureLines, id: \.self) { l in
                            Text(l)
                                .font(Theme.mono)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                        }
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(.quaternary, lineWidth: 1)
            )
            .onChange(of: lines.count) { _, _ in
                proxy.scrollTo("bottom", anchor: .bottom)
            }
        }
    }
}

/// The verbatim demo-mode label (plan.md §2).
struct DemoLabelBadge: View {
    var body: some View {
        Text(demoModeLabel)
            .font(Theme.monoSmall)
            .foregroundStyle(.secondary)
            .padding(.vertical, 3)
            .padding(.horizontal, 8)
            .background(.ultraThinMaterial, in: Capsule())
            .textSelection(.enabled)
    }
}

/// may_mutate flip indicator (§21.2): shows the side's current authority.
struct MutationBadge: View {
    let side: String
    let may: Bool?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 5) {
            Text(side).font(Theme.monoSmall).foregroundStyle(.secondary)
            Text(may == nil ? "…" : (may! ? "may_mutate=true (owner)" : "may_mutate=false (read-only)"))
                .font(Theme.monoSmall)
                .foregroundStyle(may == true ? Theme.ok : (may == nil ? Theme.pending : Theme.skipped))
                .padding(.vertical, 2)
                .padding(.horizontal, 6)
                .background(may == true ? Theme.ok.opacity(0.12) : Color.secondary.opacity(0.10),
                            in: Capsule())
        }
        .animation(carryonSpring(reduceMotion: reduceMotion), value: may)
    }
}
