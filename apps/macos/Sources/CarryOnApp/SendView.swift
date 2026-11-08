/*
 * SendView.swift — the guided send flow: choose a file → prepare (seal a
 * cut) → choose a device → send → opened. Same engine calls as the
 * Transfer Console (CarryModel.prepareSession / transfer); this pane only
 * changes presentation. Raw console + integrity toggles live under
 * "Technical details".
 */

import AppKit
import CarryOnKit
import SwiftUI
import UniformTypeIdentifiers

struct SendView: View {
    @Environment(AppState.self) private var app
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var dropTargeted = false
    @State private var showManual = false

    var body: some View {
        @Bindable var model = app.carry
        let p = app.sendPresentation

        PageScroll {
            PageHeader("Send",
                       subtitle: "Continue a file on another device — verified end to end.",
                       systemImage: "paperplane")

            Card(padding: DS.Space.m) {
                StepTrack(steps: p.steps)
            }

            HStack(alignment: .top, spacing: DS.Space.l) {
                fileCard(p)
                    .frame(maxWidth: .infinity)
                destinationCard(p, carry: model)
                    .frame(maxWidth: .infinity)
            }

            if p.running || p.verified != nil || p.error != nil {
                progressCard(p)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }

            TechnicalDetails {
                Toggle("Integrity negative test (tamper one byte — must be refused)",
                       isOn: $model.integrityNegative)
                DetailRow(label: "This Mac pin", value: app.localPinShort, monospaced: true)
                if let ev = p.evidenceClass {
                    DetailRow(label: "Evidence class", value: ev, monospaced: true)
                }
                EngineConsole(lines: model.consoleLines,
                              showFooter: model.lastResult != nil || model.lastError != nil)
                    .frame(minHeight: 180)
                Button("Open Transfer Console") { app.pane = .carry }
                    .buttonStyle(.link)
            }

            // Room for the floating action bar.
            Color.clear.frame(height: 64)
        }
        .navigationTitle("Send")
        .animation(reduceMotion ? nil : .smooth(duration: 0.3), value: p.running)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            actionBar(p)
        }
        .onAppear { if !UIFixtures.isActive { app.discovery.startBrowsing() } }
        .onDisappear { if !UIFixtures.isActive { app.discovery.stopBrowsing() } }
    }

    // MARK: File

    @ViewBuilder
    private func fileCard(_ p: SendPresentation) -> some View {
        CardSection("File", systemImage: "doc") {
            VStack(alignment: .leading, spacing: DS.Space.m) {
                dropZone(p)

                if !p.prepared.isEmpty {
                    Text("Prepared sessions")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                    VStack(spacing: DS.Space.xs) {
                        ForEach(p.prepared) { item in
                            preparedRow(item, selected: item.id == p.selectedPreparedID)
                        }
                    }
                }

                HStack {
                    Button {
                        Task { await app.carry.prepareSession() }
                    } label: {
                        Label(p.preparing ? "Preparing…" : "Prepare", systemImage: "shippingbox")
                    }
                    .secondaryActionStyle()
                    .disabled(!p.canPrepare)
                    .help("Seal a cut of this file so it can be continued elsewhere (§16.2 CutSealed)")
                    if p.preparing { ProgressView().controlSize(.small) }
                    Spacer()
                    Text("Up to 16 MiB").font(.caption).foregroundStyle(.tertiary)
                }
            }
        }
    }

    @ViewBuilder
    private func dropZone(_ p: SendPresentation) -> some View {
        let shape = RoundedRectangle(cornerRadius: DS.Radius.m, style: .continuous)
        Button(action: pickFile) {
            HStack(spacing: DS.Space.m) {
                if let path = app.carry.fileURL?.path, !UIFixtures.isActive {
                    Image(nsImage: NSWorkspace.shared.icon(forFile: path))
                        .resizable()
                        .frame(width: 44, height: 44)
                } else {
                    GlyphTile(systemImage: p.fileName == nil ? "arrow.down.doc" : "doc.richtext", size: 44)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(p.fileName ?? "Drop a file here")
                        .font(.headline)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(p.fileDetail ?? "or click to choose…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer()
                if p.fileName != nil {
                    Text("Change…").font(.callout).foregroundStyle(.tint)
                }
            }
            .padding(DS.Space.m)
            .frame(maxWidth: .infinity, minHeight: 76, alignment: .leading)
            .background(dropTargeted ? DS.brand.opacity(0.12) : Color.primary.opacity(0.03), in: shape)
            .overlay(
                shape.strokeBorder(dropTargeted ? DS.brand : DS.hairline,
                                   style: StrokeStyle(lineWidth: dropTargeted ? 2 : 1,
                                                      dash: p.fileName == nil ? [6, 4] : []))
            )
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first, url.isFileURL else { return false }
            app.carry.fileURL = url
            return true
        } isTargeted: { dropTargeted = $0 }
        .accessibilityLabel(p.fileName.map { "File \($0). Choose another file" } ?? "Choose a file")
    }

    private func preparedRow(_ item: PreparedItem, selected: Bool) -> some View {
        Button {
            if !UIFixtures.isActive { app.carry.selectedArmedID = item.id }
        } label: {
            HStack(spacing: DS.Space.s) {
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                VStack(alignment: .leading, spacing: 1) {
                    Text(item.fileName).font(.callout.weight(.medium)).lineLimit(1)
                    Text(item.detail).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                StatusPill(text: item.state.rawValue, tone: item.state.tone)
            }
            .padding(.horizontal, DS.Space.s)
            .padding(.vertical, DS.Space.xs + 2)
            .background(selected ? DS.brand.opacity(0.08) : .clear,
                        in: RoundedRectangle(cornerRadius: DS.Radius.s, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            if let s = app.carry.armedSessions.first(where: { $0.id == item.id }) {
                Button("Discard", role: .destructive) { app.carry.discard(s) }
            }
        }
    }

    // MARK: Destination

    @ViewBuilder
    private func destinationCard(_ p: SendPresentation, carry: CarryModel) -> some View {
        @Bindable var model = carry
        CardSection("Send to", systemImage: "laptopcomputer.and.iphone", accessory: {
            if !UIFixtures.isActive && app.discovery.browsing && p.devices.isEmpty {
                ProgressView().controlSize(.small)
            }
        }) {
           

[Truncated after 190 lines. To continue, call readFile again with offset=191.]