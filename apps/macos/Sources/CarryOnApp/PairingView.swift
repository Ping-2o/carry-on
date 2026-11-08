/*
 * PairingView.swift — the stage-5 PIN/QR pairing screen (spec §18.4):
 * destination arm (QR + six-digit code), source join (payload paste / typed
 * code — the honest fallback: NO camera scanner in this pilot), the mutual
 * verification-string confirm, and the §18.4.9 revoke UX.
 *
 * Labels (§2 law): the run's evidence class derives from the dialled address
 * (loopback = LOCAL; LAN = PHYSICAL) — never a static claim. No platform is
 * "supported" (§30 unrun).
 */

import CarryOnKit
import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI

struct PairingView: View {
    @Environment(AppState.self) private var app

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header
                roleRow
                destinationRow
                sourceRow
                confirmRow
                resultRow
                devicesRow
                consoleRow
            }
            .padding(20)
            .frame(maxWidth: 900, alignment: .leading)
        }
        .navigationTitle("Pairing")
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Device Pairing — PIN/QR (§18.4)").font(.title3.weight(.semibold))
            Text("The destination shows a QR + six-digit code; the source enters/pastes pairing data.\nBoth screens must show the SAME verification string — confirm on BOTH devices (§18.4.4-5).\nDISCLOSURE: pilot UI — camera scan NOT implemented (paste/type is the honest fallback); no platform is supported (§30 unrun).")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
        }
    }

    private var roleRow: some View {
        HStack(spacing: 12) {
            Button("Show my QR (destination)") { app.pairing.reset(); app.pairing.armDestination(topology: app.topology) }
                .disabled(app.pairing.role != .idle || app.topology.localIdentity == nil)
            Button("Join a device (source)") { app.pairing.reset(); app.pairing.role = .source }
                .disabled(app.pairing.role != .idle && app.pairing.role != .source)
            Spacer()
        }
    }

    @ViewBuilder
    private var destinationRow: some View {
        if let armed = app.pairing.armed {
            GroupBox {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 20) {
                        if let qr = qrImage(armed.payloadURI) {
                            Image(nsImage: qr)
                                .interpolation(.none)
                                .resizable()
                                .scaledToFit()
                                .frame(width: 240, height: 240)
                        } else {
                            Text("QR render failed — use the payload text")
                                .font(Theme.monoSmall)
                                .foregroundStyle(Theme.failed)
                        }
                        VStack(alignment: .leading, spacing: 6) {
                            Text("DESTINATION — SCAN OR READ").font(Theme.monoSmall).foregroundStyle(.secondary)
                            Text("addr:  \(armed.displayAddr)").font(Theme.mono)
                            Text("pin:   \(String(armed.pinHex.prefix(16)))…").font(Theme.mono)
                            Text("code:  \(armed.code)").font(Theme.mono)
                            Text("scan the QR (a phone camera reads it) or copy the payload text:")
                                .font(Theme.monoSmall).foregroundStyle(.secondary)
                            Text(armed.payloadURI)
                                .font(Theme.monoSmall)
                                .textSelection(.enabled)
                            Button("Copy payload") {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(armed.payloadURI, forType: .string)
                            }
                            if app.pairing.awaiting == nil {
                                Button("Accept pairing") { app.pairing.awaitPairing(topology: app.topology) }
                            } else {
                                Text("negotiating…").font(Theme.monoSmall).foregroundStyle(Theme.pending)
                            }
                        }
                    }
                }
                .padding(6)
            }
        }
    }

    @ViewBuilder
    private var sourceRow: some View {
        if app.pairing.role == .source {
            GroupBox {
                @Bindable var model = app.pairing
                VStack(alignment: .leading, spacing: 8) {
                    Text("SOURCE — ENTER PAIRING DATA").font(Theme.monoSmall).foregroundStyle(.secondary)
                    HStack(spacing: 12) {
                        Text("Addr").font(Theme.monoSmall).frame(width: 110, alignment: .leading)
                        TextField("192.168.3.14:54321", text: $model.srcAddr)
                            .textFieldStyle(.roundedBorder).font(Theme.mono).frame(width: 220)
                    }
                    HStack(spacing: 12) {
                        Text("Code").font(Theme.monoSmall).frame(width: 110, alignment: .leading)
                        TextField("six digits", text: $model.srcCode)
                            .textFieldStyle(.roundedBorder).font(Theme.mono).frame(width: 120)
                        Text("payload text (QR path, optional):")
                            .font(Theme.monoSmall).foregroundStyle(.secondary)
                    }
                    TextField("carryon://pair?v=1&…", text: $model.srcPayload, axis: .vertical)
                        .textFieldStyle(.roundedBorder).font(Theme.mono)
                        .lineLimit(2...4)
                    Text("With the payload: the destination is pinned BEFORE connecting (a MITM fails the handshake).\nWithout: provisional connect — compare BOTH verification strings.")
                        .font(Theme.monoSmall).foregroundStyle(.secondary)
                    Button("Connect & pair") { app.pairing.joinDestination(topology: app.topology) }
                        .disabled(model.srcAddr.isEmpty || model.srcCode.count != 6 || app.pairing.awaiting != nil)
                }
                .padding(6)
            }
        }
    }

    @ViewBuilder
    private var confirmRow: some View {
        if case .awaitingConfirm(let name, let pin, let vs)? = app.pairing.awaiting {
            GroupBox {
                VStack(alignment: .leading, spacing: 8) {
                    Text("VERIFY — both devices must show the SAME string")
                        .font(Theme.monoSmall).foregroundStyle(.secondary)
                    Text("peer:   \(name)").font(Theme.mono)
                    Text("pin:    \(String(pin.prefix(16)))…").font(Theme.mono)
                    Text("STRING: \(vs)").font(Theme.mono)
                    HStack {
                        Button("Confirm") { app.pairing.confirmTap(true) }
                            .keyboardShortcut(.return, modifiers: [.command])
                        Button("Decline") { app.pairing.confirmTap(false) }
                    }
                }
                .padding(6)
            }
        }
    }

    @ViewBuilder
    private var resultRow: some View {
        if let r = app.pairing.result {
            Text(r).font(Theme.monoSmall).foregroundStyle(Theme.ok).textSelection(.enabled)
        }
    }

    private var devicesRow: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                Text("Paired devices (§18.4) — revoke blocks reconnect (§18.4.9)")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
                if app.topology.records.isEmpty {
                    Text("none yet").font(Theme.monoSmall).foregroundStyle(Theme.pending)
                }
                ForEach(app.topology.records) { r in
                    HStack {
                        Text("\(r.device_name)  \(String(r.pin_hex.prefix(12)))…  gen \(r.trust_generation)\(r.revoked ? "  [REVOKED]" : "")")
                            .font(Theme.monoSmall)
                            .foregroundStyle(r.revoked ? Theme.failed : .primary)
                            .textSelection(.enabled)
                        Spacer()
                        if !r.revoked {
                            Button("Revoke") { app.topology.revoke(r) }
                        }
                    }
                }
            }
            .padding(6)
        }
    }

    private var consoleRow: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 2) {
                Text("console").font(Theme.monoSmall).foregroundStyle(.secondary)
                ForEach(Array(app.pairing.events.enumerated()), id: \.offset) { _, line in
                    Text(line).font(Theme.monoSmall).foregroundStyle(.secondary)
                }
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// QR image via the system's CIQRCodeGenerator (no third-party deps).
    private func qrImage(_ payload: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(payload.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        // Scale up (the generator is ~1px per module) with nearest-neighbor.
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        let rep = NSCIImageRep(ciImage: scaled)
        let image = NSImage(size: rep.size)
        image.addRepresentation(rep)
        return image
    }
}
