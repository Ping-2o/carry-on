/*
 * ActivityView.swift — everything you carried: sent, received, prepared and
 * demo sessions, filterable. §16.2 state names are shown verbatim next to a
 * plain-language explanation (claim-label law: never the bare word "Ready").
 */

import CarryOnKit
import SwiftUI

struct ActivityView: View {
    @Environment(AppState.self) private var app
    @State private var filter: Filter = .all
    @State private var query = ""

    enum Filter: String, CaseIterable, Identifiable {
        case all = "All"
        case transfers = "Transfers"
        case prepared = "Prepared"
        case demo = "Demo"
        var id: String { rawValue }
    }

    private func matches(_ item: SessionItem) -> Bool {
        let byFilter: Bool
        switch filter {
        case .all: byFilter = true
        case .transfers: byFilter = item.role == .sent || item.role == .received
        case .prepared: byFilter = item.role == .prepared
        case .demo: byFilter = item.role == .demoSource || item.role == .demoDestination
        }
        guard byFilter else { return false }
        guard !query.isEmpty else { return true }
        return item.title.localizedCaseInsensitiveContains(query)
            || item.subtitle.localizedCaseInsensitiveContains(query)
    }

    var body: some View {
        let all = app.sessionItems
        let items = all.filter(matches)

        PageScroll {
            PageHeader("Activity",
                       subtitle: "Files and sessions you carried between devices.",
                       systemImage: "clock.arrow.circlepath") {
                Picker("Filter", selection: $filter) {
                    ForEach(Filter.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 320)
            }

            HStack(spacing: DS.Space.m) {
                MetricTile(title: "Opened on another device",
                           value: "\(all.filter { $0.state == .actionReady || $0.state == .readOnlyActive }.count)",
                           systemImage: "checkmark.seal", tone: .success)
                MetricTile(title: "Waiting to send",
                           value: "\(all.filter { $0.state == .cutSealed }.count)",
                           systemImage: "shippingbox", tone: .info)
                MetricTile(title: "Failed or inconclusive",
                           value: "\(all.filter { $0.state == .failed || $0.state == .inconclusive }.count)",
                           systemImage: "exclamationmark.triangle", tone: .warning)
            }

            Card(padding: DS.Space.s) {
                if items.isEmpty {
                    EmptyStateView(all.isEmpty ? "No activity yet" : "No matches",
                                   message: all.isEmpty
                                       ? "Send a file or run the engine demo — sessions appear here with their exact state."
                                       : "Try a different filter or search.",
                                   systemImage: all.isEmpty ? "tray" : "magnifyingglass") {
                        if all.isEmpty {
                            Button("Send a File…") { app.pane = .send }
                        }
                    }
                } else {
                    VStack(spacing: 0) {
                        ForEach(items) { item in
                            HStack(alignment: .center) {
                                SessionRow(item: item)
                            }
                            .padding(.horizontal, DS.Space.s)
                            .help(item.state.explanation)
                            if item.id != items.last?.id { Divider().padding(.leading, 54) }
                        }
                    }
                }
            }

            StateLegend()
        }
        .navigationTitle("Activity")
        .searchable(text: $query, placement: .toolbar, prompt: "Search activity")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    app.newSession()
                } label: {
                    Label("New Session", systemImage: "plus")
                }
                .disabled(app.demo.running)
                .help("Reset the demo session (⌘N)")
            }
        }
    }
}

/// Explains the §16.2 lifecycle names users will see in pills.
private struct StateLegend: View {
    private let states: [SessionState] = [.cutSealed, .importing, .actionReady, .readOnlyActive, .failed, .inconclusive]

    var body: some View {
        TechnicalDetails("What do the states mean?") {
            Grid(alignment: .leading, horizontalSpacing: DS.Space.m, verticalSpacing: DS.Space.s) {
                ForEach(states, id: \.self) { s in
                    GridRow {
                        StatusPill(text: s.rawValue, tone: s.tone)
                        Text(s.explanation)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}
