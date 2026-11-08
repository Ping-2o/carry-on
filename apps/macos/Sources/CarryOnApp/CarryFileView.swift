/*
 * CarryFileView.swift — the "Carry File" screen (stage 3, L0/L1 onboarding
 * ladder, manual-IP mode): NSOpenPanel file pick (last directory cached),
 * hand-entered receiver address (cached; discovery is stage 4), the receiver's
 * pin, the honest L0/L1 ladder, and the engine console + dynamic disclosure.
 *
 * Labels (§2 law): a run's evidence class is derived from the dialled address
 * (loopback → LOCAL, LAN → PHYSICAL) and shown per run — never a static claim.
 */

import AppKit
import CarryOnKit
import SwiftUI

struct CarryFileView: View {
    @Environment(AppState.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The model for read-only sub-views; properties needing bindings make
    /// their own `@Bindable var model` (DemoView's pattern).
    private var model: CarryModel { app.carry }

    var body: some View {
        @Bindable var model = app.carry
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header
                fileRow
                receiverRow
                identityRow
                advancedRow
                ladderRow
                countersRow
                if let err = model.lastError {
                    Text("handoff failed:  \(err)")
                        .font(Theme.monoSmall)
                        .foregroundStyle(Theme.failed)
                        .textSelection(.enabled)
                }
                EngineConsole(lines: model.consoleLines,
                              showFooter: model.lastResult != nil || model.lastError != nil)
                    .frame(minHeight: 220)
            }
            .padding(20)
            .frame(maxWidth: 900, alignment: .leading)
        }
        .navigationTitle("Carry File")
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("Continue a file — L0/L1 ladder").font(.title3.weight(.semibold))
                Spacer()
                Button {
                    Task { await model.run() }
                } label: {
                    Label(model.running ? "Serving…" : "Serve to receiver", systemImage: "paperplane")
                }
                .keyboardShortcut("s", modifiers: [.command, .shift])
                .disabled(model.running || !model.configValid)
            }
            Text("Manual IP until discovery (stage 4): the receiver binds a listener and shows its\naddress + pin; you dial it here. Chunk size is destination-controlled (§18.6) — set it on the receiver.")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
            Text("L0 = file carried + verified · L1 = opened in a real app (on the receiver). Honest per-run label below the console.")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
        }
    }

    /// The picked file — NSOpenPanel with the last chosen directory restored.
    private var fileRow: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Button("Choose file…") { pickFile() }
                        .disabled(model.running)
                    if let url = model.fileURL {
                        Text(url.lastPathComponent)
                            .font(Theme.mono)
                            .textSelection(.enabled)
                        Spacer()
                        Text(ByteCountFormatter.string(fromByteCount:
                                Int64((try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0),
                                countStyle: .file))
                            .font(Theme.monoSmall)
                            .foregroundStyle(.secondary)
                    } else {
                        Spacer()
                        Text("no file chosen")
                            .font(Theme.monoSmall)
                            .foregroundStyle(Theme.pending)
                    }
                }
            }
            .padding(6)
        }
    }

    private var receiverRow: some View {
        GroupBox {
            @Bindable var model = model
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    Text("Receiver").font(Theme.monoSmall).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                    TextField("192.168.1.20:48961", text: $model.receiverAddr)
                        .textFieldStyle(.roundedBorder)
                        .font(Theme.mono)
                        .frame(width: 220)
                        .disabled(model.running)
                    Text(model.addrValid ? "IPv4:port ✓" : "IPv4:port required")
                        .font(Theme.monoSmall)
                        .foregroundStyle(model.addrValid ? Theme.ok : Theme.pending)
                }
                HStack(spacing: 12) {
                    Text("Receiver pin").font(Theme.monoSmall).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                    TextField("64-hex pin from its Receive screen", text: $model.receiverPin)
                        .textFieldStyle(.roundedBorder)
                        .font(Theme.mono)
                        .frame(width: 220)
                        .disabled(model.running)
                    Text(model.pinValid ? "64 hex ✓" : "64 hex required")
                        .font(Theme.monoSmall)
                        .foregroundStyle(model.pinValid ? Theme.ok : Theme.pending)
                }
                Text("The receiver's pin is a certificate fingerprint (public, not a secret) — type or paste the full 64 hex.")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
            }
            .padding(6)
        }
    }

    /// This Mac's pre-armed identity: truncated for display (§22.3 style), the
    /// full pin available to the CLIPBOARD only (the receiver needs it in full
    /// to pin this Mac — manual mode's honest human channel).
    private var identityRow: some View {
        HStack(spacing: 8) {
            Text("this Mac's pin:  \(model.sourcePinTruncated.isEmpty ? "…" : model.sourcePinTruncated)")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            Button("Copy full pin") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(model.sourcePinHex, forType: .string)
            }
            .disabled(model.sourcePinHex.isEmpty)
            Button("Re-arm") { model.rearmIdentity() }
                .disabled(model.running)
            Spacer()
            if let s = model.sessionState {
                Text("state:  \(s.rawValue)")
                    .font(Theme.monoSmall)
                    .foregroundStyle(Theme.accent)
                    .textSelection(.enabled)
            }
        }
    }

    @ViewBuilder
    private var advancedRow: some View {
        @Bindable var model = model
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                Toggle("Integrity negative test (CORE-004)", isOn: $model.integrityNegative)
                    .disabled(model.running)
                Text("declares a TAMPERED content hash — the receiver's gate must REFUSE (visible FAIL, fail closed);\nthe engine still carries and verifies the real bytes against the manifest.")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
            }
            .padding(6)
        }
    }

    /// The honest ladder: L0 done here when the receiver reports ImportComplete;
    /// L1 runs ON THE RECEIVER (its engine document.open + OS ACTION_VIEW) —
    /// never claimed as done on this screen.
    private var ladderRow: some View {
        HStack(spacing: 20) {
            ForEach(HandoffLadderStep.allCases) { step in
                LadderCell(step: step, outcome: model.ladderStates[step], onReceiver: step == .l1Open)
            }
            Spacer()
        }
    }

    private var countersRow: some View {
        HStack(spacing: 18) {
            ByteBlocks(recv: model.bytesRecv, active: model.running)
            ByteCounter(label: "sent", value: model.bytesSent)
            ByteCounter(label: "recv", value: model.bytesRecv)
            Spacer()
            if let r = model.lastResult {
                Text(String(format: "done:  %.1f ms, verify ok=%@ (%@)",
                            r.elapsedMS, r.verifyOK ? "true" : "false", r.verifyMessage))
                    .font(Theme.monoSmall)
                    .foregroundStyle(r.verifyOK ? Theme.ok : Theme.failed)
                    .textSelection(.enabled)
            }
            if let cls = model.evidenceClass {
                Text(cls == "physical" ? "OBSERVED (PHYSICAL)" : "OBSERVED (LOCAL)")
                    .font(Theme.monoSmall)
                    .foregroundStyle(cls == "physical" ? Theme.wire : Theme.pending)
                    .textSelection(.enabled)
            }
        }
    }

    private func pickFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose the file to continue on the receiver (≤ 16 MiB, the file adapter's object limit)"
        if let last = UserDefaults.standard.url(forKey: "carryon.handoff.lastDir") {
            panel.directoryURL = last // cached last chosen directory
        }
        if panel.runModal() == .OK, let url = panel.url {
            model.fileURL = url
            UserDefaults.standard.set(url.deletingLastPathComponent(),
                                      forKey: "carryon.handoff.lastDir")
        }
    }
}

/// One ladder step cell: `▪ L0 ─ file carried + verified`, with the honest
/// on-receiver half-step for L1 (◐ — done on the receiver, verifiable there).
struct LadderCell: View {
    let step: HandoffLadderStep
    let outcome: StageOutcome?
    let onReceiver: Bool

    private var glyph: String {
        switch outcome {
        case .ok: return "▪"
        case .skipped: return "◐"
        case .failed: return "✗"
        case nil: return "▫"
        }
    }

    private var color: Color {
        switch outcome {
        case .ok: return Theme.ok
        case .skipped: return Theme.skipped
        case .failed: return Theme.failed
        case nil: return Theme.pending
        }
    }

    var body: some View {
        HStack(spacing: 4) {
            Text(glyph).font(Theme.mono).foregroundStyle(color)
            Text(step.label).font(Theme.monoSmall)
                .foregroundStyle(outcome == nil ? Theme.pending : .primary)
            if onReceiver {
                Text("(on receiver)").font(Theme.monoSmall).foregroundStyle(Theme.skipped)
            }
        }
        .textSelection(.enabled)
    }
}
