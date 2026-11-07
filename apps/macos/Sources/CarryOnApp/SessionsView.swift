/*
 * SessionsView.swift — the session list with the exact §16.2 states (never the
 * bare word "Ready"), authority_epoch, and may_mutate (plan.md §3).
 */

import CarryOnKit
import SwiftUI

struct SessionsView: View {
    @Environment(AppState.self) private var app

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if app.demo.sourceSessions.isEmpty && app.demo.destSessions.isEmpty {
                    Text("no sessions yet — run the demo (⌘D) to create + carry one")
                        .font(Theme.monoSmall)
                        .foregroundStyle(.secondary)
                        .padding(.top, 30)
                } else {
                    sessionTable
                }
            }
            .padding(20)
            .frame(maxWidth: 900, alignment: .leading)
        }
        .navigationTitle("Sessions")
    }

    private var sessionTable: some View {
        GroupBox("Sessions (spec §16.2 states)") {
            VStack(alignment: .leading, spacing: 10) {
                Text("SOURCE").font(Theme.monoSmall).foregroundStyle(.secondary)
                ForEach(app.demo.sourceSessions, id: \.id) { s in
                    sessionRow(id: s.id, state: s.state,
                               may: app.demo.mayMutateSource,
                               epoch: nil,
                               origin: "this Mac, carryon-core #1")
                }
                Text("DESTINATION (mirror)").font(Theme.monoSmall).foregroundStyle(.secondary)
                ForEach(app.demo.destSessions, id: \.id) { s in
                    sessionRow(id: s.id, state: s.state,
                               may: app.demo.mayMutateDest,
                               epoch: app.demo.lastResult?.newEpoch,
                               origin: "imported mirror of the source session")
                }
                Text("States are the exact §16.2 lifecycle states. Bare \"Ready\" is not one of them — readiness is ActionReady, ownership is the authority epoch.")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func sessionRow(id: String, state: SessionState, may: Bool?,
                            epoch: UInt64?, origin: String) -> some View {
        HStack(spacing: 12) {
            Text(String(id.prefix(8)) + "…").font(Theme.mono)
            stateBadge(state)
            if let epoch {
                Text("authority_epoch=\(epoch)").font(Theme.monoSmall).foregroundStyle(.secondary)
            }
            MutationBadge(side: "may_mutate", may: may)
            Spacer()
            Text(origin).font(Theme.monoSmall).foregroundStyle(.secondary)
        }
        .textSelection(.enabled)
    }

    private func stateBadge(_ state: SessionState) -> some View {
        let color: Color = switch state {
        case .actionReady: Theme.ok
        case .readOnlyActive: Theme.wire
        case .failed, .aborted: Theme.failed
        case .inconclusive: Theme.skipped
        default: Theme.accent
        }
        return Text(state.rawValue)
            .font(Theme.monoSmall)
            .foregroundStyle(color)
            .padding(.vertical, 2)
            .padding(.horizontal, 7)
            .background(color.opacity(0.12), in: Capsule())
    }
}
