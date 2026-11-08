/*
 * HomeView.swift — the landing pane: one clear call to action (continue a
 * file on another device), what's nearby, and what happened recently.
 */

import CarryOnKit
import SwiftUI

struct HomeView: View {
    @Environment(AppState.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let nearby = app.nearbyDevices
        let paired = app.pairedDevices.filter { $0.status == .paired }
        let sessions = app.sessionItems

        PageScroll {
            HeroCard(reduceMotion: reduceMotion,
                     onSend: { app.pane = .send },
                     onPair: { app.pane = .pairing })

            HStack(spacing: DS.Space.m) {
                MetricTile(title: "Nearby devices", value: "\(nearby.count)", systemImage: "wifi")
                MetricTile(title: "Paired devices", value: "\(paired.count)",
                           systemImage: "checkmark.shield", tone: .success)
                MetricTile(title: "Sessions", value: "\(sessions.count)",
                           systemImage: "clock.arrow.circlepath", tone: .neutral)
            }

            HStack(alignment: .top, spacing: DS.Space.l) {
                CardSection("Nearby", systemImage: "dot.radiowaves.left.and.right", accessory: {
                    Button("All Devices") { app.pane = .devices }
                        .buttonStyle(.link)
                }) {
                    if nearby.isEmpty {
                        EmptyStateView("Looking for devices…",
                                       message: "Open Carry-On on your phone or another Mac on the same network.",
                                       systemImage: "antenna.radiowaves.left.and.right",
                                       showsProgress: !reduceMotion)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(nearby.prefix(4)) { device in
                                DeviceRow(device: device) {
                                    Button {
                                        app.carry.receiverAddr = device.address ?? ""
                                        app.pane = .send
                                    } label: {
                                        Image(systemName: "paperplane.fill")
                                    }
                                    .buttonStyle(.borderless)
                                    .help("Send to \(device.name)")
                                }
                                if device.id != nearby.prefix(4).last?.id { Divider() }
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity)

                CardSection("Recent activity", systemImage: "clock", accessory: {
                    Button("Show All") { app.pane = .activity }
                        .buttonStyle(.link)
                }) {
                    if sessions.isEmpty {
                        EmptyStateView("Nothing carried yet",
                                       message: "Files and sessions you send or receive show up here.",
                                       systemImage: "tray")
                    } else {
                        VStack(spacing: 0) {
                            ForEach(sessions.prefix(4)) { item in
                                SessionRow(item: item)
                                if item.id != sessions.prefix(4).last?.id { Divider() }
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity)
            }

            TipRow()
        }
        .navigationTitle("Home")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    app.pane = .send
                } label: {
                    Label("Send File", systemImage: "paperplane")
                }
                .help("Send a file to another device")
            }
        }
        .onAppear { if !UIFixtures.isActive { app.discovery.startBrowsing() } }
        .onDisappear { if !UIFixtures.isActive { app.discovery.stopBrowsing() } }
    }
}

private struct HeroCard: View {
    let reduceMotion: Bool
    let onSend: () -> Void
    let onPair: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: DS.Space.xl) {
            VStack(alignment: .leading, spacing: DS.Space.m) {
                Text("Continue on another device")
                    .font(.system(size: 30, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Start here, pick up there. Carry-On moves a file or a live session to a nearby device — verified end to end, over your own network.")
                    .font(.title3)
                    .foregroundStyle(.white.opacity(0.88))
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: DS.Space.s) {
                    Button(action: onSend) {
                        Label("Send a File…", systemImage: "paperplane.fill")
                            .padding(.horizontal, DS.Space.xs)
                    }
                    .controlSize(.large)
                    .buttonStyle(.borderedProminent)
                    .tint(.white)
                    .foregroundStyle(DS.brandDeep)
                    .keyboardShortcut(.defaultAction)

                    Button(action: onPair) {
                        Label("Pair a Device", systemImage: "qrcode")
                    }
                    .controlSize(.large)
                    .buttonStyle(.bordered)
                    .tint(.white)
                }
                .padding(.top, DS.Space.xs)
            }
            Spacer(minLength: DS.Space.l)
            HandoffIllustration(animate: !reduceMotion)
                .frame(width: 230, height: 150)
        }
        .padding(DS.Space.xl + 4)
        .background(
            ZStack {
                DS.brandGradient
                RadialGradient(colors: [.white.opacity(0.22), .clear],
                               center: .topTrailing, startRadius: 10, endRadius: 360)
            }
        )
        .clipShape(RoundedRectangle(cornerRadius: DS.Radius.xl, style: .continuous))
        .shadow(color: DS.brandDeep.opacity(0.25), radius: 16, y: 8)
    }
}

/// Laptop → phone illustration built from SF Symbols.
private struct HandoffIllustration: View {
    let animate: Bool

    var body: some View {
        ZStack {
            Circle()
                .fill(.white.opacity(0.10))
                .frame(width: 150, height: 150)
            HStack(spacing: DS.Space.m) {
                Image(systemName: "laptopcomputer")
                    .font(.system(size: 54, weight: .regular))
                Image(systemName: "arrow.right")
                    .font(.system(size: 22, weight: .bold))
                    .symbolEffect(.pulse, options: .repeating, isActive: animate)
                Image(systemName: "iphone")
                    .font(.system(size: 46, weight: .regular))
            }
            .foregroundStyle(.white)
        }
        .accessibilityHidden(true)
    }
}

private struct TipRow: View {
    var body: some View {
        HStack(spacing: DS.Space.s) {
            Image(systemName: "lightbulb")
                .foregroundStyle(.yellow)
            Text("Tip: on the receiving device, open Carry-On → Receive. Paired devices connect without typing a pin.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, DS.Space.xs)
    }
}
