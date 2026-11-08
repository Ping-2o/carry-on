/*
 * CarryFileView.swift — the "Transfer session" screen (stage 6): PREPARE an
 * armed session («активная сессия», §16.2 CutSealed) from a chosen file, then
 * TRANSFER it to the ENGINE-DISCOVERED receiver (stage 4) using the PAIRED
 * Keychain trust (stage 5) — the manual typed-pin path stays the fallback.
 * The honest L0/L1 ladder + engine console + dynamic disclosure live below.
 *
 * Labels (§2 law): a run's evidence class is derived from the dialled address
 * (loopback → LOCAL, LAN → PHYSICAL) and shown per run — never a static claim.
 * Interrupted ≠ success (§19.3.9): a failed leg never shows as transferred.
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
                sessionsRow
                receiverRow
                identityRow
                advancedRow
                ladderRow
                countersRow
                if let err = model.lastError {
                    Text("transfer failed:  \(err)")
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
        .navigationTitle("Transfer Session")
        // Foreground-first (battery): browse only while this screen is up.
        .onAppear { app.discovery.startBrowsing() }
        .onDisappear { app.discovery.stopBrowsing() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("Transfer a session — L0/L1 cross-device").font(.title3.weight(.semibold))
                Spacer()
                Button {
                    Task { await model.prepareSession() }
                } label: {
                    Label(model.preparing ? "Preparing…" : "1 · Prepare session", systemImage: "doc.badge.plus")
                }
                .disabled(model.preparing || !model.prepareValid)
                Button {
                    Task { await model.transfer() }
                } label: {
                    Label(model.running ? "Transferring…" : "2 · Transfer to receiver", systemImage: "paperplane")
                }
                .keyboardShortcut("s", modifiers: [.command, .shift])
                .disabled(model.running || !model.transferValid)
            }
            Text("1 · Prepare seals a cut (an \"активная сессия\", §16.2 CutSealed — the engine session waits, transferable).\n2 · Transfer discovers receivers on the LAN (_carryon._tcp, §18.3 — the engine validates the TXT), picks one, and\nserves over TLS 1.3 with the PAIRED trust (stage 5; unpaired/revoked fails the handshake closed §18.5).\nChunk size is destination-controlled (§18.6) — the receiver sets it.")
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
                        .disabled(model.preparing || model.running)
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

    /// The armed sessions («активные сессии») — §16.2 CutSealed, selectable.
    private var sessionsRow: some View {
        GroupBox("Active sessions (§16.2 CutSealed)") {
            VStack(alignment: .leading, spacing: 8) {
                if model.armedSessions.isEmpty {
                    Text("none yet — pick a file and press «1 · Prepare session»")
                        .font(Theme.monoSmall)
                        .foregroundStyle(Theme.pending)
                } else {
                    ForEach(model.armedSessions) { s in
                        let selected = model.selectedArmedID == s.id
                        Button {
                            model.selectedArmedID = s.id
                        } label: {
                            HStack(spacing: 8) {
                                Text(selected ? "◉" : "○").font(Theme.mono)
                                    .foregroundStyle(selected ? Theme.wire : Theme.pending)
                                Text(s.title).font(Theme.mono)
                                Text(s.subtitle).font(Theme.monoSmall).foregroundStyle(.secondary)
                                Spacer()
                                stateBadge(s.state)
                                if let r = s.servedResult {
                                    Text(r.serveLegsUsed > 1
                                         ? "transferred (\(r.serveLegsUsed) legs, resumed §11.2)"
                                         : "transferred")
                                        .font(Theme.monoSmall).foregroundStyle(Theme.ok)
                                }
                                Button("Discard") { model.discard(s) }
                                    .controlSize(.small)
                                    .disabled(model.running || s.servedResult != nil)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(model.running)
                    }
                }
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func stateBadge(_ s: SessionState) -> some View {
        // §16.2 names are the law — never the bare word "Ready".
        Text(s.rawValue)
            .font(Theme.monoSmall)
            .foregroundStyle(s == .actionReady ? Theme.ok : (s == .failed ? Theme.failed : Theme.accent))
    }

    private var receiverRow: some View {
        GroupBox {
            @Bindable var model = model
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    Text("Receiver").font(Theme.monoSmall).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                    Text(app.discovery.browsing ? "discovering (engine browse)…" : "browse off")
                        .font(Theme.monoSmall)
                        .foregroundStyle(app.discovery.browsing ? Theme.wire : Theme.pending)
                    Spacer()
                    if let ms = app.discovery.firstPeerLatencyMs {
                        Text(String(format: "NET-003: first peer %.0f ms", ms))
                            .font(Theme.monoSmall)
                            .foregroundStyle(ms < 10_000 ? Theme.ok : Theme.failed)
                            .textSelection(.enabled)
                    }
                }
                // ---- engine-discovered receivers (§18.3) ----
                if app.discovery.peers.isEmpty {
                    Text("no receivers discovered yet — make sure the receiver's app is on its Receive screen (it advertises); or dial it manually below")
                        .font(Theme.monoSmall)
                        .foregroundStyle(Theme.pending)
                } else {
                    ForEach(app.discovery.peers) { peer in
                        let selected = model.receiverAddr == peer.address
                        Button {
                            model.receiverAddr = peer.address
                            // NET-003 rides with the run (monotonic, §18.10).
                            model.discoveryLatencyMS = app.discovery.firstPeerLatencyMs
                        } label: {
                            HStack(spacing: 8) {
                                Text(selected ? "◉" : "○").font(Theme.mono)
                                    .foregroundStyle(selected ? Theme.wire : Theme.pending)
                                Text(peer.instanceName).font(Theme.mono)
                                Text("dc=\(peer.txt.device_class) pr=\(peer.txt.pairing_required ? "1" : "0")")
                                    .font(Theme.monoSmall).foregroundStyle(.secondary)
                                Spacer()
                                Text(peer.address).font(Theme.monoSmall).foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(model.running)
                    }
                }
                // ---- trust: the PAIRED store (stage 5) or the typed pin ----
                HStack(spacing: 12) {
                    Toggle("Paired trust (stage-5 Keychain store)", isOn: $model.usePairedTrust)
                        .disabled(model.running)
                    Text(model.usePairedTrust
                         ? (model.hasPairedRecords ? "pinned records found" : "NO paired records — pair first or fall back")
                         : "manual fallback")
                        .font(Theme.monoSmall)
                        .foregroundStyle(model.usePairedTrust
                                         ? (model.hasPairedRecords ? Theme.ok : Theme.failed)
                                         : Theme.pending)
                }
                if !model.usePairedTrust {
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
                }
                // ---- manual address fallback (stage-3 path, kept) ----
                HStack(spacing: 12) {
                    Text("Manual").font(Theme.monoSmall).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                    TextField("192.168.1.20:48961", text: $model.receiverAddr)
                        .textFieldStyle(.roundedBorder)
                        .font(Theme.mono)
                        .frame(width: 220)
                        .disabled(model.running)
                    Text(model.addrValid ? "IPv4:port ✓" : "IPv4:port required")
                        .font(Theme.monoSmall)
                        .foregroundStyle(model.addrValid ? Theme.ok : Theme.pending)
                    Text("(fallback)")
                        .font(Theme.monoSmall)
                        .foregroundStyle(.secondary)
                }
                Text("Discovery yields the address; the trust is the stage-5 pairing (fail-closed §18.5) or the typed pin.\nChunk/budget are the receiver's knobs (§18.6) — see its Receive screen.")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
            }
            .padding(6)
        }
    }

    /// This Mac's DURABLE identity (stage-5 Keychain): truncated for display
    /// (§22.3 style), the full pin available to the CLIPBOARD only.
    private var identityRow: some View {
        HStack(spacing: 8) {
            Text("this Mac's pin:  \(model.durablePinTruncated)")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            Button("Copy full pin") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(model.sourcePinHex.isEmpty
                                               ? (app.topology.localPin) : model.sourcePinHex,
                                               forType: .string)
            }
            .disabled(app.topology.localPin.isEmpty)
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
                    .disabled(model.running || model.preparing)
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
                Text(String(format: "done:  %.1f ms, verify ok=%@, legs=%d, trust=%@",
                            r.elapsedMS, r.verifyOK ? "true" : "false", r.serveLegsUsed,
                            r.pairedTrust ? "paired" : "typed"))
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
