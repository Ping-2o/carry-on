/*
 * RootView.swift — the app shell: a source-list sidebar grouped by intent
 * (Carry · Apps · Developer), a detail pane, and the first-run onboarding.
 *
 * HIG: sidebar for top-level navigation, grouped sections, collapsible
 * developer tools, standard toolbar placement. On macOS 26 the sidebar and
 * toolbar float on Liquid Glass automatically (system components).
 *
 * Claim-label law (§2): the demo label stays visible in the sidebar footer.
 */

import SwiftUI

struct RootView: View {
    @Environment(AppState.self) private var app
    @AppStorage("carryon.onboarding.done") private var onboardingDone = false
    @State private var showOnboarding = false

    var body: some View {
        NavigationSplitView {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 210, ideal: 232, max: 300)
        } detail: {
            DetailView(pane: app.pane)
        }
        .frame(minWidth: 1000, minHeight: 660)
        .onAppear {
            showOnboarding = !onboardingDone && !UIFixtures.isActive
        }
        .sheet(isPresented: $showOnboarding, onDismiss: { onboardingDone = true }) {
            OnboardingView { showOnboarding = false }
        }
    }
}

private struct DetailView: View {
    let pane: AppState.Pane

    var body: some View {
        switch pane {
        case .home: HomeView()
        case .send: SendView()
        case .devices: DevicesView()
        case .activity: ActivityView()
        case .photocraft: PhotoCraftView()
        case .demo: DemoView()
        case .carry: CarryFileView()
        case .pairing: PairingView()
        case .topology: TopologyView()
        case .sessions: SessionsView()
        case .evidence: EvidenceView()
        }
    }
}

private struct SidebarView: View {
    @Environment(AppState.self) private var app
    @AppStorage("carryon.sidebar.developerExpanded") private var developerExpanded = true

    var body: some View {
        List(selection: Binding(get: { app.pane }, set: { app.pane = $0 ?? .home })) {
            Section(AppState.Pane.Group.carry.rawValue) {
                rows(for: .carry)
            }
            Section(AppState.Pane.Group.apps.rawValue) {
                rows(for: .apps)
            }
            Section(AppState.Pane.Group.developer.rawValue, isExpanded: $developerExpanded) {
                rows(for: .developer)
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            SidebarFooter()
        }
    }

    @ViewBuilder
    private func rows(for group: AppState.Pane.Group) -> some View {
        ForEach(AppState.Pane.panes(in: group)) { pane in
            NavigationLink(value: pane) {
                Label {
                    Text(pane.title)
                } icon: {
                    Image(systemName: pane.symbol)
                }
            }
            .badge(badge(for: pane))
        }
    }

    private func badge(for pane: AppState.Pane) -> Int {
        switch pane {
        case .devices: return app.nearbyDevices.count
        default: return 0
        }
    }
}

/// Compact identity + honesty footer at the bottom of the sidebar.
private struct SidebarFooter: View {
    @Environment(AppState.self) private var app

    var body: some View {
        VStack(alignment: .leading, spacing: DS.Space.s) {
            HStack(spacing: DS.Space.s) {
                GlyphTile(systemImage: "laptopcomputer", size: 28)
                VStack(alignment: .leading, spacing: 0) {
                    Text(app.localDeviceName)
                        .font(.callout.weight(.medium))
                        .lineLimit(1)
                    Text("Pin \(app.localPinShort)")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            StatusPill(text: "Demo · OBSERVED (LOCAL)", systemImage: "flask", tone: .warning)
                .help("binding IMPLEMENTED · demo OBSERVED (LOCAL) · platform supported: NO (§30 unrun)")
        }
        .padding(DS.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
