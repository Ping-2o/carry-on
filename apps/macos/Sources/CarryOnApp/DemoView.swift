/*
 * DemoView.swift — Demo Mode (plan.md §2): scenario picker, advanced toggles,
 * the animated 8-stage ASCII pipeline, live byte counters, per-stage states,
 * the may_mutate flip indicator, and the engine console + DISCLOSURE footer.
 */

import CarryOnKit
import SwiftUI

struct DemoView: View {
    @Environment(AppState.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var showAdvanced = false

    var body: some View {
        @Bindable var model = app.demo
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header
                controls
                StagePipeline(states: stageOutcomes, runningStage: model.runningStage)
                counters
                authorityRow
                if let err = model.lastError {
                    Text("run failed:  \(err)")
                        .font(Theme.monoSmall)
                        .foregroundStyle(Theme.failed)
                        .textSelection(.enabled)
                }
                EngineConsole(lines: model.consoleLines, showFooter: model.lastResult != nil || model.lastError != nil)
                    .frame(minHeight: 260)
            }
            .padding(20)
            .frame(maxWidth: 900, alignment: .leading)
        }
        .navigationTitle("Demo")
    }

    private var stageOutcomes: [DemoStage: StageOutcome] {
        var out: [DemoStage: StageOutcome] = [:]
        for (stage, state) in app.demo.stageStates {
            if let o = state.outcome { out[stage] = o }
        }
        return out
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("Demo Mode — one device only").font(.title3.weight(.semibold))
                Spacer()
                Button {
                    Task { await app.runDemo() }
                } label: {
                    Label(app.demo.running ? "Running…" : "Run demo", systemImage: "play.fill")
                }
                .keyboardShortcut("d", modifiers: .command)
                .disabled(app.demo.running)
            }
            DemoLabelBadge()
        }
    }

    private var controls: some View {
        @Bindable var model = app.demo
        return GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Picker("Scenario", selection: $model.config.scenario) {
                    ForEach(DemoScenario.allCases) { s in
                        Text(s.title).tag(s)
                    }
                }
                .pickerStyle(.radioGroup)
                .horizontalRadioGroupLayout()
                .disabled(model.running)

                Toggle("Advanced showcase", isOn: $showAdvanced.animation(carryonSpring(reduceMotion: reduceMotion)))
                if showAdvanced {
                    VStack(alignment: .leading, spacing: 8) {
                        Picker("Chunk size (§18.6)", selection: $model.config.chunkSize) {
                            Text("engine default").tag(UInt64(0))
                            Text("4 KiB").tag(UInt64(4096))
                            Text("16 KiB").tag(UInt64(16384))
                            Text("64 KiB").tag(UInt64(65536))
                        }
                        .frame(width: 320)
                        .disabled(model.running)

                        Toggle("Budget showcase (§6.8)", isOn: $model.config.budgetRejection)
                            .disabled(model.running)
                        Text("tiny total_net_bytes → the import refuses or admits a visible prefix; a NORMAL outcome, fail closed (§6.8)")
                            .font(Theme.monoSmall).foregroundStyle(.secondary)

                        Toggle("Suspend / resume (§11.2)", isOn: $model.config.suspendResume)
                            .disabled(model.running)
                        Text("pre-arm carryon_core_request_suspend → observe {suspended:true} → carryon_resume_import completes")
                            .font(Theme.monoSmall).foregroundStyle(.secondary)

                        Text("Recovery report fields (interrupted_transfers, hidden_incomplete_objects, ambiguous_sessions, journal_truncated, clean) print in the console at OPEN.")
                            .font(Theme.monoSmall).foregroundStyle(.secondary)
                    }
                    .padding(.leading, 4)
                }
            }
            .padding(6)
        }
    }

    private var counters: some View {
        HStack(spacing: 18) {
            ByteBlocks(recv: app.demo.bytesRecv, active: app.demo.running)
            ByteCounter(label: "sent", value: app.demo.bytesSent)
            ByteCounter(label: "recv", value: app.demo.bytesRecv)
            Spacer()
            if let r = app.demo.lastResult {
                Text(String(format: "done:  %.1f ms, verify ok=%@ (%@)",
                            r.elapsedMS, r.verifyOK ? "true" : "false", r.verifyMessage))
                    .font(Theme.monoSmall)
                    .foregroundStyle(r.verifyOK ? Theme.ok : Theme.failed)
                    .textSelection(.enabled)
            }
        }
    }

    private var authorityRow: some View {
        HStack(spacing: 18) {
            MutationBadge(side: "source", may: app.demo.mayMutateSource)
            MutationBadge(side: "dest", may: app.demo.mayMutateDest)
            Spacer()
            if let epoch = app.demo.lastResult?.newEpoch {
                Text("authority_epoch:  \(epoch)").font(Theme.monoSmall).foregroundStyle(.secondary)
            }
        }
    }
}
