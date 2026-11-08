/*
 * RootView.swift — NavigationSplitView shell (HIG): sidebar with ⌘1…⌘4 panes,
 * first-run onboarding gate, and the demo label in the sidebar footer.
 */

import SwiftUI

struct RootView: View {
    @Environment(AppState.self) private var app
    @AppStorage("carryon.onboarding.done") private var onboardingDone = false
    @State private var showOnboarding = false

    var body: some View {
        NavigationSplitView {
            List(selection: Binding(get: { app.pane }, set: { app.pane = $0 ?? .demo })) {
                ForEach(AppState.Pane.allCases) { pane in
                    NavigationLink(value: pane) {
                        Label(pane.rawValue, systemImage: pane.symbol)
                    }
                    .keyboardShortcut(paneShortcut(pane), modifiers: .command)
                }
            }
            .listStyle(.sidebar)
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 4) {
                    DemoLabelBadge()
                    Text("binding IMPLEMENTED · demo OBSERVED (LOCAL) · platform supported: NO (§30 unrun)")
                        .font(Theme.monoSmall)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 10)
                        .padding(.bottom, 8)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.bar)
            }
        } detail: {
            switch app.pane {
            case .demo: DemoView()
            case .carry: CarryFileView()
            case .topology: TopologyView()
            case .sessions: SessionsView()
            case .evidence: EvidenceView()
            }
        }
        .frame(minWidth: 980, minHeight: 620)
        .onAppear {
            showOnboarding = !onboardingDone
        }
        .sheet(isPresented: $showOnboarding, onDismiss: { onboardingDone = true }) {
            OnboardingView { showOnboarding = false }
        }
    }

    private func paneShortcut(_ pane: AppState.Pane) -> KeyEquivalent {
        switch pane {
        case .demo: return "1"
        case .carry: return "2"
        case .topology: return "3"
        case .sessions: return "4"
        case .evidence: return "5"
        }
    }
}
