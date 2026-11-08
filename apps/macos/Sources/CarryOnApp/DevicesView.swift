/*
 * DevicesView.swift — this Mac, devices nearby on the network, and the
 * devices you trust (paired). Pairing and revocation live here; the raw
 * topology/pairing panes stay available under Developer.
 */

import CarryOnKit
import SwiftUI

struct DevicesView: View {
    @Environment(AppState.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var pendingRevoke: DeviceItem?

    var body: some View {
        let nearby = app.nearbyDevices
        let paired = app.pairedDevices

        PageScroll {
            PageHeader("Devices",
                       subtitle: "Devices nearby and the ones you trust.",
                       systemImage: "laptopcomputer.and.iphone") {
                Button {
                    app.pane = .pairing
                } label: {
                    Label("Pair New Device", systemImage: "qrcode")
                }
                .controlSize(.large)
                .primaryActionStyle()
            }

            ThisMacCard(name: app.localDeviceName, pin: app.localPinShort,
                        advertising: UIFixtures.isActive || app.discovery.advertising)

            CardSection("Nearby", systemImage: "dot.radiowaves.left.and.right", accessory: {
                if !UIFixtures.isActive && app.discovery.browsing {
                    ProgressView().controlSize(.small)
                }
            }) {
                if nearby.isEmpty {
                    EmptyStateView("No devices nearby yet",
                                   message: "Devices running Carry-On on this network appear automatically.",
                                   systemImage: "antenna.radiowaves.left.and.right",
                                   showsProgress: !reduceMotion)
                } else {
                    VStack(spacing: 0) {
                        ForEach(nearby) { device in
                            DeviceRow(device: device) {
                                Button("Send…") {
                                    app.carry.receiverAddr = device.address ?? ""
                                    app.pane = .send
                                }
                                .controlSize(.small)
                            }
                            if device.id != nearby.last?.id { Divider() }
                        }
                    }
                }
            }

            CardSection("Trusted devices", systemImage: "checkmark.shield") {
                if paired.isEmpty {
                    EmptyStateView("No trusted devices",
                                   message: "Pair once with a QR code — later transfers connect securely without a pin.",
                                   systemImage: "key.horizontal") {
                        Button("Pair New Device") { app.pane = .pairing }
                    }
                } else {
                    VStack(spacing: 0) {
                        ForEach(paired) { device in
                            DeviceRow(device: device) {
                                if device.status == .paired {
                                    Menu {
                                        if let pin = device.pin {
                                            Text("Pin \(pin)")
                                        }
                                        Divider()
                                        Button("Revoke Trust…", role: .destructive) {
                                            pendingRevoke = device
                                        }
                                    } label: {
                                        Image(systemName: "ellipsis.circle")
                                    }
                                    .menuStyle(.borderlessButton)
                                    .menuIndicator(.hidden)
                                    .fixedSize()
                                    .help("More actions")
                                }
                            }
                            if device.id != paired.last?.id { Divider() }
                        }
                    }
                }
            }

            TechnicalDetails {
                DetailRow(label: "Service type", value: "_carryon._tcp", monospaced: true)
                DetailRow(label: "TXT contract", value: "§18.3 minimal (pm id pr dc ch)", monospaced: true)
                DetailRow(label: "Trust store", value: "Keychain (PLAT-002)", monospaced: true)
                Button("Open Topology") { app.pane = .topology }
                    .buttonStyle(.link)
            }
        }
        .navigationTitle("Devices")
        .onAppear { if !UIFixtures.isActive { app.discovery.startBrowsing() } }
        .onDisappear { if !UIFixtures.isActive { app.discovery.stopBrowsing() } }
        .confirmationDialog("Revoke trust for \(pendingRevoke?.name ?? "this device")?",
                            isPresented: Binding(get: { pendingRevoke != nil },
                                                 set: { if !$0 { pendingRevoke = nil } }),
                            titleVisibility: .visible) {
            Button("Revoke", role: .destructive) {
                if let d = pendingRevoke, !UIFixtures.isActive,
                   let record = app.topology.records.first(where: { $0.pin_hex == d.id }) {
                    app.topology.revoke(record)
                }
                pendingRevoke = nil
            }
        } message: {
            Text("Future connections from this device will be refused until you pair again.")
        }
    }
}

private struct ThisMacCard: View {
    let name: String
    let pin: String
    let advertising: Bool

    var body: some View {
        Card {
            HStack(spacing: DS.Space.l) {
                GlyphTile(systemImage: "laptopcomputer", size: 52)
                VStack(alignment: .leading, spacing: DS.Space.xxs) {
                    Text(name).font(.title3.weight(.semibold))
                    Text("Device pin \(pin)")
                        .font(.callout.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                Spacer()
                StatusPill(text: advertising ? "Visible to nearby devices" : "Not advertising",
                           systemImage: advertising ? "eye" : "eye.slash",
                           tone: advertising ? .success : .neutral)
            }
        }
    }
}
