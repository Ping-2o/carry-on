/*
 * PhotoCraftView.swift — the stage-10 desktop flagship pane: Carry-On drives a
 * LIVE PhotoCraft session through its loopback control channel, carries it to a
 * second core in visible per-tile budget windows, and restores it into a second
 * live PhotoCraft. No fork: the app is only driven through its own automation
 * roots and control methods.
 *
 * Two roles in one pane (Send / Receive) so the honest LOCAL equivalent of the
 * two-Mac story — two app instances on ONE Mac — is a two-window demo.
 *
 * Labels are law: every run is OBSERVED (LOCAL); the footer says so per run.
 *
 * Authority (#24): the pane shows the single-writer holder, offers an explicit
 * hand-over on receive, and refuses a second writer (fail closed).
 */

import AppKit
import CarryOnKit
import SwiftUI

@MainActor
@Observable
final class PhotoCraftModel {
    enum Mode: String, CaseIterable, Identifiable {
        case send = "Send"
        case receive = "Receive"
        var id: String { rawValue }
    }

    var mode: Mode = .send
    var running = false

    // Live PhotoCraft connection (persisted so a demo restart keeps the setup).
    var controlAddr: String {
        didSet { d.set(controlAddr, forKey: "carryon.photocraft.addr") }
    }
    var tokenFile: String {
        didSet { d.set(tokenFile, forKey: "carryon.photocraft.token") }
    }
    var readRoot: String {
        didSet { d.set(readRoot, forKey: "carryon.photocraft.read") }
    }
    var writeRoot: String {
        didSet { d.set(writeRoot, forKey: "carryon.photocraft.write") }
    }
    var peerAddr: String {
        didSet { d.set(peerAddr, forKey: "carryon.photocraft.peerAddr") }
    }
    var peerPin: String {
        didSet { d.set(peerPin, forKey: "carryon.photocraft.peerPin") }
    }
    var windowKB: Int {
        didSet { d.set(windowKB, forKey: "carryon.photocraft.windowKB") }
    }
    var dataDir: String
    /// Split (PhotoCraft's own tiles) or monolithic (one `.pcraft` object).
    var encoding: String {
        didSet { d.set(encoding, forKey: "carryon.photocraft.encoding") }
    }

    // Results
    private(set) var live: PhotoCraftLiveSession?
    private(set) var capture: PhotoCraftCapture?
    private(set) var progress: PhotoCraftProgress?
    private(set) var result: PhotoCraftResult?
    private(set) var listenAddr: String?
    private(set) var identityPin: String = ""
    var consoleLines: [String] = []
    var lastError: String?

    /// Single-writer authority as this pane knows it (#24). Deliberately NOT
    /// cleared by reset(): forgetting it would let a replica write again.
    private(set) var authority: PhotoCraftAuthorityState = .origin
    private(set) var authorityDetail = "no hand-over yet: a fresh local session is its own single writer"
    private(set) var authorityReceipt: String?
    /// Explicit opt-in before a receive: take authority after a verified restore.
    var takeAuthority = false
    /// Why the next send would be refused (fail closed), if it would.
    var sendRefusal: String? { PhotoCraftAuthority.refusal(.send, state: authority) }
    /// Why the next receive would be refused (fail closed), if it would.
    var receiveRefusal: String? {
        PhotoCraftAuthority.refusal(takeAuthority ? .receiveAndTakeAuthority : .receive,
                                    state: authority)
    }

    private let d = UserDefaults.standard
    private var identity: DeviceIdentity?
    private var runToken = UUID()

    init() {
        controlAddr = d.string(forKey: "carryon.photocraft.addr") ?? "127.0.0.1:7898"
        tokenFile = d.string(forKey: "carryon.photocraft.token") ?? ""
        readRoot = d.string(forKey: "carryon.photocraft.read") ?? ""
        writeRoot = d.string(forKey: "carryon.photocraft.write") ?? ""
        peerAddr = d.string(forKey: "carryon.photocraft.peerAddr") ?? "127.0.0.1:7899"
        peerPin = d.string(forKey: "carryon.photocraft.peerPin") ?? ""
        windowKB = d.object(forKey: "carryon.photocraft.windowKB") as? Int ?? 256
        encoding = d.string(forKey: "carryon.photocraft.encoding") ?? "split"
        let base = FileManager.default.urls(for: .applicationSupportDirectory,
                                            in: .userDomainMask).first?.path
            ?? NSTemporaryDirectory()
        dataDir = "\(base)/CarryOn/photocraft"
        Task { await makeIdentity() }
    }

    private func makeIdentity() async {
        do {
            // Per-run identity (a fingerprint the peer pins by hand). The
            // durable Keychain identity is the stage-5 pairing path; this pane
            // keeps the manual pin flow visible and honest.
            let id = try await Task.detached { try DeviceIdentity.generate(name: "carryon-mac-photocraft") }.value
            identity = id
            identityPin = try id.pinHex()
        } catch {
            lastError = error.localizedDescription
        }
    }

    var liveConfig: PhotoCraftLiveConfig {
        PhotoCraftLiveConfig(addr: controlAddr, tokenFile: tokenFile,
                             readRoot: readRoot, writeRoot: writeRoot)
    }
    var configValid: Bool { liveConfig.isValid }
    /// A pin is always needed; only the receiver dials, so only it needs an address.
    var pinValid: Bool { HandoffValidation.isValidPinHex(peerPin) }
    var receivePeerValid: Bool {
        HandoffValidation.isValidReceiverAddr(peerAddr) && pinValid
    }
    var windowBytes: UInt64 { UInt64(max(windowKB, 4)) * 1024 }

    func reset() {
        guard !running else { return }
        live = nil; capture = nil; progress = nil; result = nil
        listenAddr = nil; consoleLines = []; lastError = nil
    }

    /// Read the live app (one inspect; nothing is written).
    func inspect() async {
        guard configValid else {
            lastError = "Fill the live PhotoCraft control address, token file and both automation roots."
            return
        }
        lastError = nil
        do {
            let cfg = liveConfig
            let session = try await Task.detached { try PhotoCraftBridge.inspect(cfg) }.value
            live = session
            consoleLines.append("inspect: \(session.name) · \(session.documents) docs · rev \(session.revision) · dirty \(session.dirty)")
        } catch {
            lastError = error.localizedDescription
        }
    }

    func send() async {
        guard !running, configValid, pinValid, let identity else { return }
        if let why = sendRefusal { lastError = "refused: \(why)"; return }
        reset()
        running = true
        let token = UUID()
        runToken = token
        defer { if runToken == token { running = false } }
        let cfg = PhotoCraftSendConfig(
            live: liveConfig, receiverPin: peerPin,
            dataDir: dataDir + "/send", windowBytes: windowBytes,
            encoding: encoding, identity: identity)
        do {
            _ = try await Task.detached { [weak self] in
                try PhotoCraftSender.run(cfg) { [weak self] event in
                    let s = self
                    Task { @MainActor in s?.apply(event, token: token) }
                }
            }.value
        } catch {
            guard runToken == token else { return }
            lastError = error.localizedDescription
            if authority == .handingOver {
                authority = .ambiguous
                authorityDetail = "the run failed during a hand-over; refused until §21.3 recovery"
            }
        }
    }

    func receive() async {
        guard !running, configValid, receivePeerValid, let identity else { return }
        if let why = receiveRefusal { lastError = "refused: \(why)"; return }
        reset()
        running = true
        let token = UUID()
        runToken = token
        defer { if runToken == token { running = false } }
        let cfg = PhotoCraftReceiveConfig(
            live: liveConfig, sourceAddr: peerAddr, sourcePin: peerPin,
            dataDir: dataDir + "/receive", windowBytes: windowBytes,
            identity: identity, takeAuthority: takeAuthority)
        do {
            _ = try await Task.detached { [weak self] in
                try PhotoCraftReceiver.run(cfg) { [weak self] event in
                    let s = self
                    Task { @MainActor in s?.apply(event, token: token) }
                }
            }.value
        } catch {
            guard runToken == token else { return }
            lastError = error.localizedDescription
            if authority == .handingOver {
                authority = .ambiguous
                authorityDetail = "the run failed during a hand-over; refused until §21.3 recovery"
            }
        }
    }

    private func apply(_ event: PhotoCraftEvent, token: UUID) {
        guard token == runToken else { return }
        switch event {
        case .console(let line): consoleLines.append(line)
        case .live(let l): live = l
        case .capture(let c): capture = c
        case .listening(let addr, let pin):
            listenAddr = addr
            if identityPin.isEmpty { identityPin = pin }
        case .descriptor: break
        case .progress(let p): progress = p
        case .served: break
        case .authority(let a):
            authority = a.state
            authorityDetail = a.detail
            authorityReceipt = a.receiptJSON
            consoleLines.append("authority: \(a.state.rawValue) · may_mutate \(a.mayMutate) · \(a.detail)")
        case .finished(let r): result = r
        case .failed(let why): lastError = why
        }
    }
}

struct PhotoCraftView: View {
    @Environment(AppState.self) private var app
    private var model: PhotoCraftModel { app.photocraft }

    var body: some View {
        @Bindable var model = app.photocraft
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header(model)
                liveConfigBox(model)
                liveCard
                rolePicker(model)
                authorityBox(model)
                if model.mode == .send { sendBox(model) } else { receiveBox(model) }
                if let p = model.progress { progressBox(p, model: model) }
                if let r = model.result { resultBox(r) }
                if let err = model.lastError {
                    Text("photocraft carry failed:  \(err)")
                        .font(Theme.monoSmall)
                        .foregroundStyle(Theme.failed)
                        .textSelection(.enabled)
                }
                consoleBox
                footer
            }
            .padding(20)
            .frame(maxWidth: 940, alignment: .leading)
        }
        .navigationTitle("PhotoCraft")
        .onAppear {
            // A demo checkout often has a live app already: probe it without
            // writing anything so the pane's card is real, not illustrative.
            if model.configValid, model.live == nil {
                Task { await model.inspect() }
            }
        }
    }

    private func header(_ model: PhotoCraftModel) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("PhotoCraft — live session carry").font(.title3.weight(.semibold))
                Spacer()
                Text("OBSERVED (LOCAL)")
                    .font(Theme.monoSmall)
                    .foregroundStyle(Theme.wire)
            }
            Text("Carry-On drives the RUNNING PhotoCraft over its loopback JSON control channel (no MCP, no fork),\nseals a cut through org.carryon.photocraft, streams it in visible §6.8 budget windows, and reopens it\nin a second live PhotoCraft. Send and Receive here are two instances on one Mac — not two machines.")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
        }
    }

    private func liveConfigBox(_ model: PhotoCraftModel) -> some View {
        @Bindable var model = model
        return GroupBox("Live PhotoCraft (control channel, loopback)") {
            VStack(alignment: .leading, spacing: 8) {
                field("control", $model.controlAddr, placeholder: "127.0.0.1:7898")
                field("token file", $model.tokenFile, placeholder: "…/a.token (0600, never displayed)")
                field("read root", $model.readRoot, placeholder: "automation --automation-read-root")
                field("write root", $model.writeRoot, placeholder: "automation --automation-write-root")
                HStack {
                    Button("Inspect live session") { Task { await model.inspect() } }
                        .disabled(model.running || !model.configValid)
                    if let live = model.live {
                        Text("\(live.name) · rev \(live.revision) · \(live.dirty ? "dirty" : "clean") · \(live.tool) · zoom \(String(format: "%.2f", live.zoom)) · \(live.selectedLayers) layer(s)")
                            .font(Theme.monoSmall)
                            .foregroundStyle(.secondary)
                    }
                }
            }.padding(6)
        }
    }

    private var liveCard: some View {
        Group {
            if let live = model.live {
                GroupBox("Live session") {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("\(live.name)  (document \(live.active + 1) of \(live.documents))")
                            .font(Theme.monoTitle)
                        Text("revision \(live.revision) · \(live.dirty ? "unsaved edits present" : "fully saved") · tool \(live.tool) · zoom \(String(format: "%.3f", live.zoom)) · rotation \(String(format: "%.2f", live.rotation)) · \(live.selectedLayers) selected layer(s)")
                            .font(Theme.monoSmall)
                            .foregroundStyle(.secondary)
                    }.padding(6).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    private func rolePicker(_ model: PhotoCraftModel) -> some View {
        @Bindable var model = model
        return Picker("Role", selection: $model.mode) {
            ForEach(PhotoCraftModel.Mode.allCases) { Text($0.rawValue).tag($0) }
        }
        .pickerStyle(.segmented)
        .disabled(model.running)
    }

    private func authorityBox(_ model: PhotoCraftModel) -> some View {
        GroupBox("Authority: single writer (L4)") {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text("holder").font(Theme.monoSmall).foregroundStyle(.secondary)
                        .frame(width: 92, alignment: .leading)
                    Text(model.authority.holder)
                        .font(Theme.mono)
                        .foregroundStyle(authorityColor(model.authority))
                }
                Text(model.authorityDetail)
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
                    .textSelection(.enabled)
                if let receipt = model.authorityReceipt {
                    Text("receipts: \(receipt)")
                        .font(Theme.monoSmall).foregroundStyle(.secondary)
                        .lineLimit(2).truncationMode(.middle)
                        .textSelection(.enabled)
                }
                Text("Carry-On refuses its own writes when it is not the single writer (fail closed). PhotoCraft itself is not locked (no fork), so edits typed directly into a replica's window are outside this guarantee.")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
            }.padding(6).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func authorityColor(_ s: PhotoCraftAuthorityState) -> Color {
        switch s {
        case .origin, .thisInstance: return Theme.ok
        case .peer, .handingOver: return Theme.wire
        case .conflict, .ambiguous: return Theme.failed
        }
    }

    private func sendBox(_ model: PhotoCraftModel) -> some View {
        @Bindable var model = model
        return GroupBox("Send to a second instance") {
            VStack(alignment: .leading, spacing: 8) {
                Text("This side LISTENS; the receiver dials the address shown after you start. Enter the receiver's pin first (it pins the connecting client).")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
                field("receiver pin", $model.peerPin, placeholder: "64-hex fingerprint from the receiver")
                HStack(spacing: 10) {
                    Stepper("window \(model.windowKB) KiB", value: $model.windowKB, in: 16...8192, step: 16)
                        .font(Theme.monoSmall)
                    Picker("encoding", selection: $model.encoding) {
                        Text("split (PhotoCraft tiles)").tag("split")
                        Text("monolithic (.pcraft)").tag("monolithic")
                    }.frame(width: 240)
                    Spacer()
                    Button {
                        Task { await model.send() }
                    } label: {
                        Label(model.running ? "Serving…" : "Cut & send session", systemImage: "paperplane.fill")
                    }
                    .disabled(model.running || !model.configValid || !model.pinValid || model.sendRefusal != nil)
                }
                if let why = model.sendRefusal {
                    Text("send refused: \(why)").font(Theme.monoSmall).foregroundStyle(Theme.failed)
                }
                Text("If the receiver asks for authority, this instance hands it over on the wire and becomes a read-only replica.")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
                if let addr = model.listenAddr {
                    HStack(spacing: 8) {
                        Text("listening \(addr)").font(Theme.mono).textSelection(.enabled)
                        Button("copy addr") { copy(addr) }.font(Theme.monoSmall)
                        Text("this side's pin \(model.identityPin.prefix(12))…").font(Theme.monoSmall)
                            .foregroundStyle(.secondary)
                        Button("copy pin") { copy(model.identityPin) }.font(Theme.monoSmall)
                    }
                }
            }.padding(6)
        }
    }

    private func receiveBox(_ model: PhotoCraftModel) -> some View {
        @Bindable var model = model
        return GroupBox("Receive from a sending instance") {
            VStack(alignment: .leading, spacing: 8) {
                field("source addr", $model.peerAddr, placeholder: "127.0.0.1:PORT (from the sender's card)")
                field("source pin", $model.peerPin, placeholder: "64-hex fingerprint")
                HStack(spacing: 10) {
                    Stepper("window \(model.windowKB) KiB", value: $model.windowKB, in: 16...8192, step: 16)
                        .font(Theme.monoSmall)
                    Spacer()
                    Button {
                        Task { await model.receive() }
                    } label: {
                        Label(model.running ? "Receiving…" : "Receive & restore", systemImage: "tray.and.arrow.down.fill")
                    }
                    .disabled(model.running || !model.configValid || !model.receivePeerValid || model.receiveRefusal != nil)
                }
                Toggle("Hand over authority to this instance after a verified restore (explicit, L4)",
                       isOn: $model.takeAuthority)
                    .font(Theme.monoSmall)
                    .disabled(model.running)
                if let why = model.receiveRefusal {
                    Text("receive refused: \(why)").font(Theme.monoSmall).foregroundStyle(Theme.failed)
                }
                Text("this side's pin \(model.identityPin.prefix(12))…  (copy it to the sender)")
                    .font(Theme.monoSmall).foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }.padding(6)
        }
    }

    private func progressBox(_ p: PhotoCraftProgress, model: PhotoCraftModel) -> some View {
        GroupBox("Progressive transfer — §6.8 windows") {
            VStack(alignment: .leading, spacing: 6) {
                ProgressView(value: p.fraction)
                Text("objects \(p.publishedObjects)/\(p.totalObjects) · windows \(p.windows) · window \(p.windowBytes / 1024) KiB\(p.widenedWindows > 0 ? " (widened \(p.widenedWindows)×)" : "") · \(p.wireBytes) B on the wire")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
                Text("Each window is the engine's real budget admission; published objects are verified + published and skipped by dedup on the next window. The document is usable only after the closure completes (fail closed).")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
            }.padding(6)
        }
    }

    private func resultBox(_ r: PhotoCraftResult) -> some View {
        GroupBox("Result") {
            VStack(alignment: .leading, spacing: 4) {
                Text("\(r.role) · \(r.name) · \(r.objects) objects · \(r.windows) windows · \(r.wireBytes) B · \(String(format: "%.2f", r.seconds)) s")
                    .font(Theme.mono)
                if let restore = r.restore {
                    Text("opened in PhotoCraft: \(restore.restored.name) · tool \(restore.restored.tool) · zoom \(String(format: "%.2f", restore.restored.zoom)) · \(restore.restored.selectedLayers) layer(s) · oracle \(restore.oracle.agreed ? "AGREED" : "DISAGREED")")
                        .font(Theme.monoSmall)
                        .foregroundStyle(restore.oracle.agreed ? Theme.ok : Theme.failed)
                    Text(restore.oracle.detail).font(Theme.monoSmall).foregroundStyle(.secondary)
                }
                Text("label: \(r.label) — two app instances on one Mac, loopback; NOT a physical or §30 claim")
                    .font(Theme.monoSmall).foregroundStyle(Theme.wire)
            }.padding(6).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var consoleBox: some View {
        GroupBox("Console") {
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(model.consoleLines.enumerated()), id: \.offset) { _, line in
                        Text(line).font(Theme.monoSmall).textSelection(.enabled)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(minHeight: 120, maxHeight: 220).padding(6)
        }
    }

    private var footer: some View {
        Text("Channel: PhotoCraft's loopback JSON control channel (bearer token read from its 0600 file inside the engine; never shown). The adapter is compiled in; PhotoCraft is never forked. Wording: prototype / platform pilot — no platform is \"supported\" until §30 passes on hardware.")
            .font(Theme.monoSmall)
            .foregroundStyle(.secondary)
    }

    private func field(_ label: String, _ binding: Binding<String>, placeholder: String) -> some View {
        HStack(spacing: 10) {
            Text(label).font(Theme.monoSmall).foregroundStyle(.secondary)
                .frame(width: 92, alignment: .leading)
            TextField(placeholder, text: binding)
                .textFieldStyle(.roundedBorder)
                .font(Theme.mono)
        }
    }

    private func copy(_ s: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(s, forType: .string)
    }
}
