/*
 * EvidenceView.swift — the evidence bundle viewer (plan.md §4). Parses the
 * EvidenceBundle JSON (evidence.rs field names) and shows section hashes, the
 * journal timeline, metrics, and the disclosure verbatim. Bundle content is
 * displayed ONLY — never executed (EVD-005).
 */

import CarryOnKit
import SwiftUI

struct EvidenceView: View {
    @Environment(AppState.self) private var app
    @State private var bundle: EvidenceBundle?
    @State private var parseError: String?
    @State private var showImporter = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header
                if let bundle {
                    bundleView(bundle)
                } else if let parseError {
                    Text("parse failed:  \(parseError)").font(Theme.monoSmall).foregroundStyle(Theme.failed)
                } else {
                    Text("no bundle loaded — run the demo (⌘D) or load a bundle JSON")
                        .font(Theme.monoSmall).foregroundStyle(.secondary)
                }
            }
            .padding(20)
            .frame(maxWidth: 900, alignment: .leading)
        }
        .navigationTitle("Evidence")
        .fileImporter(isPresented: $showImporter, allowedContentTypes: [.json]) { result in
            loadFile(result)
        }
        .onAppear { reload() }
        .onChange(of: app.evidenceJSON) { _, _ in reload() }
    }

    private func reload() {
        guard let json = app.evidenceJSON else { return }
        if let b = EvidenceBundle.parse(json) {
            bundle = b
            parseError = nil
        } else {
            parseError = "not an EvidenceBundle (EVD-005: parsed, never executed)"
            bundle = nil
        }
    }

    private func loadFile(_ result: Result<URL, Error>) {
        guard case .success(let url) = result else { return }
        let secured = url.startAccessingSecurityScopedResource()
        defer { if secured { url.stopAccessingSecurityScopedResource() } }
        guard let text = try? String(contentsOf: url, encoding: .utf8) else {
            parseError = "unreadable file"
            return
        }
        app.evidenceJSON = text
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text("Evidence bundle (§23.3)").font(.title3.weight(.semibold))
                Spacer()
                Button("Load JSON…") { showImporter = true }
                if let r = app.demo.lastResult {
                    Button("Load last demo bundle") {
                        app.evidenceJSON = r.bundleJSON
                    }
                    .disabled(r.bundleJSON.isEmpty)
                }
            }
            Text("Verify only parses and hash-checks; it NEVER executes bundle content (EVD-005).")
                .font(Theme.monoSmall).foregroundStyle(.secondary)
        }
    }

    private func bundleView(_ b: EvidenceBundle) -> some View {
        GroupBox("bundle") {
            VStack(alignment: .leading, spacing: 10) {
                LabeledRow(key: "bundle_version", value: "\(b.bundle_version)")
                LabeledRow(key: "session_id", value: b.session_id)
                LabeledRow(key: "created_utc", value: b.created_utc)
                LabeledRow(key: "verifier_version", value: b.verifier_version)
                LabeledRow(key: "build", value: "crate=\(b.build.crate_version) schema=\(b.build.schema_version) phase=\(b.build.phase)")

                sectionHashes(b)
                journal(b)
                actions(b)
                metrics(b)

                Divider()
                Text("disclosure (verbatim):").font(Theme.monoSmall).foregroundStyle(.secondary)
                Text(b.disclosure)
                    .font(Theme.mono)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            .padding(6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func sectionHashes(_ b: EvidenceBundle) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("section_hashes:").font(Theme.monoSmall).foregroundStyle(.secondary)
            ForEach(b.section_hashes) { h in
                HStack(spacing: 8) {
                    Text("▪ \(h.name)").font(Theme.mono).frame(width: 90, alignment: .leading)
                    Text(h.sha256).font(Theme.monoSmall).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }
        }
    }

    private func journal(_ b: EvidenceBundle) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("journal timeline (\(b.journal.count) events):").font(Theme.monoSmall).foregroundStyle(.secondary)
            ForEach(Array(b.journal.enumerated()), id: \.offset) { _, e in
                let etype = (e.stringValue == nil)
                    ? (caseObject(e)["event_type"]?.stringValue ?? "event")
                    : (e.stringValue ?? "event")
                let utc = caseObject(e)["utc"]?.stringValue ?? ""
                let rc = caseObject(e)["result_code"]?.stringValue ?? ""
                Text("  · \(utc)  \(etype)  \(rc)")
                    .font(Theme.monoSmall)
                    .textSelection(.enabled)
            }
        }
    }

    private func actions(_ b: EvidenceBundle) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("actions (\(b.actions.count)):").font(Theme.monoSmall).foregroundStyle(.secondary)
            ForEach(Array(b.actions.enumerated()), id: \.offset) { _, a in
                let cls = caseObject(a)["class"]?.stringValue ?? "action"
                let oracle = caseObject(a)["oracle"]?.stringValue ?? ""
                Text("  · \(cls)  oracle=\(oracle)")
                    .font(Theme.monoSmall)
                    .textSelection(.enabled)
            }
        }
    }

    private func metrics(_ b: EvidenceBundle) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("metrics:").font(Theme.monoSmall).foregroundStyle(.secondary)
            if case .object(let o) = b.metrics {
                ForEach(o.keys.sorted(), id: \.self) { k in
                    Text("  \(k):  \(o[k]?.rendered ?? "—")")
                        .font(Theme.monoSmall)
                        .textSelection(.enabled)
                }
            }
        }
    }

    private func caseObject(_ v: AnyJSON) -> [String: AnyJSON] {
        if case .object(let o) = v { return o }
        return [:]
    }
}
