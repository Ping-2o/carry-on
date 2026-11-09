/*
 * PhotoCraft.swift — stage-10 desktop flagship UX (Carry-On side).
 *
 * The live-app bridge: Carry-On captures a running PhotoCraft desktop session
 * over PhotoCraft's **loopback JSON control channel** (not MCP — see
 * docs/artcraft-bridge/UX.md), seals a cut through the compiled-in
 * `org.carryon.photocraft` adapter, streams it to a second core in the engine's
 * real §6.8 budget windows (the per-tile progress the UI shows), and restores
 * the session into a second live PhotoCraft by opening the carried bytes in it
 * and reapplying the working position.
 *
 * NEVER forks PhotoCraft: the app is driven only through its own automation
 * roots and control methods; the adapter reads/writes the user's selected
 * document workspace.
 *
 * LABELS (§2 law): every run here is OBSERVED (LOCAL) — loopback control
 * channels and loopback TLS on one machine. A second Mac is not involved, so
 * nothing is PHYSICAL and no platform becomes "supported" (§30 unrun).
 *
 * AUTHORITY (#24, L4 §21.2): the pane shows who holds single-writer authority
 * and offers an explicit hand-over (the engine's serve/request authority
 * transfer on one more hop). Carry-On refuses its own writes when it is not the
 * single writer: two-writer attempts fail closed (PhotoCraftAuthority.refusal).
 *
 * Secrets (EVD-006): the control bearer token is passed to the engine as a
 * FILE PATH only; it is read inside Rust and never returned. This layer never
 * logs it and never displays it. Pins are certificate fingerprints.
 */

import CCarryon
import Foundation

// ---- the live control-channel config (data only) ------------------------------

public struct PhotoCraftLiveConfig: Sendable, Equatable {
    /// `127.0.0.1:<port>` of the running app's `--control` listener.
    public var addr: String
    /// Path to the app's 0600 bearer-token file (never its contents).
    public var tokenFile: String
    /// The app's `--automation-read-root` / `--automation-write-root`.
    public var readRoot: String
    public var writeRoot: String

    public init(addr: String, tokenFile: String, readRoot: String, writeRoot: String) {
        self.addr = addr
        self.tokenFile = tokenFile
        self.readRoot = readRoot
        self.writeRoot = writeRoot
    }

    /// Validation for the UI: loopback only (the control channel is loopback by
    /// design), all paths present, token file readable.
    public var isValid: Bool {
        let host = String(addr.prefix(while: { $0 != ":" }))
        guard HandoffValidation.isValidReceiverAddr(addr),
              host == "127.0.0.1" || host == "localhost",
              !tokenFile.isEmpty, !readRoot.isEmpty, !writeRoot.isEmpty,
              FileManager.default.isReadableFile(atPath: tokenFile),
              FileManager.default.fileExists(atPath: readRoot),
              FileManager.default.fileExists(atPath: writeRoot)
        else { return false }
        return true
    }
}

/// The engine-side config JSON. The token travels as a path (EVD-006).
func photocraftConfigJSON(_ live: PhotoCraftLiveConfig,
                          encoding: String? = nil,
                          workspaceJSON: String? = nil,
                          nameHint: String? = nil,
                          screenshot: String? = nil) -> String {
    var o: [String: Any] = [
        "addr": live.addr,
        "token_file": live.tokenFile,
        "read_root": live.readRoot,
        "write_root": live.writeRoot,
    ]
    if let encoding { o["encoding"] = encoding }
    if let workspaceJSON { o["workspace_json"] = workspaceJSON }
    if let nameHint { o["name_hint"] = nameHint }
    if let screenshot { o["screenshot"] = screenshot }
    let data = (try? JSONSerialization.data(withJSONObject: o)) ?? Data("{}".utf8)
    return String(data: data, encoding: .utf8) ?? "{}"
}

// ---- engine results (all decode from the append-only JSON shapes) ------------

/// `carryon_photocraft_inspect` output — the live-session card.
public struct PhotoCraftLiveSession: Codable, Sendable, Equatable {
    public var live: Bool
    public var addr: String
    public var documents: Int
    public var active: Int
    public var name: String
    public var path: String?
    public var revision: UInt64
    public var dirty: Bool
    public var tool: String
    public var zoom: Double
    public var rotation: Double
    public var selectedLayers: Int

    enum CodingKeys: String, CodingKey {
        case live, addr, documents, active, name, path, revision, dirty, tool, zoom, rotation
        case selectedLayers = "selected_layers"
    }
}

/// `carryon_photocraft_register` output — what was captured into the cut.
public struct PhotoCraftCapture: Codable, Sendable, Equatable {
    public var adapterID: String
    public var encoding: String
    public var name: String
    public var revision: UInt64
    public var dirty: Bool
    public var path: String?
    public var parts: Int
    public var documentBytes: UInt64
    public var unsavedBytes: UInt64
    public var totalBytes: UInt64
    public var tool: String
    public var zoom: Double
    public var rotation: Double
    public var selectedLayers: Int
    public var workspaceJSON: String

    enum CodingKeys: String, CodingKey {
        case encoding, name, revision, dirty, path, parts, tool, zoom, rotation
        case adapterID = "adapter_id"
        case documentBytes = "document_bytes"
        case unsavedBytes = "unsaved_bytes"
        case totalBytes = "total_bytes"
        case selectedLayers = "selected_layers"
        case workspaceJSON = "workspace_json"
    }
}

/// `carryon_photocraft_restore` output — the live restore + engine oracle.
public struct PhotoCraftRestoreReport: Codable, Sendable, Equatable {
    public struct Restored: Codable, Sendable, Equatable {
        public var openedPath: String
        public var active: Int
        public var name: String
        public var revision: UInt64
        public var dirty: Bool
        public var tool: String
        public var zoom: Double
        public var rotation: Double
        public var selectedLayers: Int
        public var screenshot: String?
        public var screenshotError: String?

        enum CodingKeys: String, CodingKey {
            case active, name, revision, dirty, tool, zoom, rotation, screenshot
            case openedPath = "opened_path"
            case selectedLayers = "selected_layers"
            case screenshotError = "screenshot_error"
        }
    }
    public struct Oracle: Codable, Sendable, Equatable {
        public var checked: Bool
        public var agreed: Bool
        public var detail: String
    }
    public var restored: Restored
    public var liveRestore: Bool
    public var oracle: Oracle
    public var workspace: String
    public var objects: Int
    public var bytes: UInt64
    public var adapterID: String

    enum CodingKeys: String, CodingKey {
        case restored, oracle, workspace, objects, bytes
        case liveRestore = "live_restore"
        case adapterID = "adapter_id"
    }
}

// ---- ABI wrappers ------------------------------------------------------------

public enum PhotoCraftBridge {
    /// Inspect the live app (one `session.inspect` + one `ui.inspect`); writes nothing.
    public static func inspect(_ live: PhotoCraftLiveConfig) throws -> PhotoCraftLiveSession {
        let json = try outString("carryon_photocraft_inspect") { out in
            photocraftConfigJSON(live).withCString { carryon_photocraft_inspect($0, out) }
        }
        guard let s = try? JSONDecoder().decode(PhotoCraftLiveSession.self, from: Data(json.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable photocraft inspect: \(json.prefix(300))")
        }
        return s
    }

    /// Capture the live session into the engine's PhotoCraft adapter.
    public static func register(_ engine: Engine, _ live: PhotoCraftLiveConfig,
                                encoding: String, sessionTitle: String) throws -> PhotoCraftCapture {
        let o: [String: Any] = [
            "addr": live.addr, "token_file": live.tokenFile,
            "read_root": live.readRoot, "write_root": live.writeRoot,
            "encoding": encoding, "session_title": sessionTitle,
        ]
        let data = (try? JSONSerialization.data(withJSONObject: o)) ?? Data("{}".utf8)
        let cfg = String(data: data, encoding: .utf8) ?? "{}"
        let json = try cfg.withCString { c in
            try outString("carryon_photocraft_register") { out in
                carryon_photocraft_register(engine.handle, c, out)
            }
        }
        guard let c = try? JSONDecoder().decode(PhotoCraftCapture.self, from: Data(json.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable photocraft capture: \(json.prefix(300))")
        }
        return c
    }

    /// Restore an imported session into a second live PhotoCraft (destination side).
    public static func restore(_ engine: Engine, _ live: PhotoCraftLiveConfig,
                               workspaceJSON: String, nameHint: String,
                               mirrorSession: String, mirrorCut: UInt64,
                               screenshot: String? = nil) throws -> PhotoCraftRestoreReport {
        let cfg = photocraftConfigJSON(live, workspaceJSON: workspaceJSON,
                                       nameHint: nameHint, screenshot: screenshot)
        let json = try cfg.withCString { c in
            try mirrorSession.withCString { s in
                try outString("carryon_photocraft_restore") { out in
                    carryon_photocraft_restore(engine.handle, c, s, mirrorCut, out)
                }
            }
        }
        guard let r = try? JSONDecoder().decode(PhotoCraftRestoreReport.self, from: Data(json.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable photocraft restore: \(json.prefix(400))")
        }
        return r
    }
}

// ---- the transfer descriptor (Hello features; data only, §3.8) ----------------

public struct PhotoCraftDescriptor: Sendable, Equatable {
    public static let marker = "org.carryon.photocraft.ux.v1"
    public static let done = "org.carryon.photocraft.ux.done=1"

    public var session: String
    public var cut: UInt64
    public var objects: Int
    public var name: String
    public var dirty: Bool
    public var workspaceJSON: String

    public init(session: String, cut: UInt64, objects: Int, name: String,
                dirty: Bool, workspaceJSON: String) {
        self.session = session
        self.cut = cut
        self.objects = objects
        self.name = name
        self.dirty = dirty
        self.workspaceJSON = workspaceJSON
    }

    /// The source's Welcome features. Every field is data (ids, a count, a name,
    /// a flag, a JSON fragment) — nothing executable (§3.8).
    public var features: [String] {
        [
            Self.marker,
            "session=\(session)",
            "cut=\(cut)",
            "objects=\(objects)",
            "name=\(name)",
            "dirty=\(dirty)",
            "workspace=\(workspaceJSON)",
        ]
    }

    /// Strict parse of the peer's features; nil (fail closed) without the marker
    /// or a required field.
    public static func parse(features: [String]) -> PhotoCraftDescriptor? {
        guard features.contains(marker) else { return nil }
        var values: [String: String] = [:]
        for f in features {
            guard let eq = f.firstIndex(of: "=") else { continue }
            let key = String(f[f.startIndex..<eq])
            let value = String(f[f.index(after: eq)...])
            if values[key] == nil { values[key] = value }
        }
        guard let session = values["session"], !session.isEmpty,
              let cut = values["cut"].flatMap(UInt64.init),
              let objects = values["objects"].flatMap(Int.init), objects > 0,
              let name = values["name"], !name.isEmpty,
              let dirty = values["dirty"].map({ $0 == "true" }),
              let workspace = values["workspace"], workspace.hasPrefix("{")
        else { return nil }
        return PhotoCraftDescriptor(session: session, cut: cut, objects: objects,
                                    name: name, dirty: dirty, workspaceJSON: workspace)
    }

    public static func isDone(features: [String]) -> Bool {
        features.contains(done)
    }
}

// ---- import outcome + progressive progress (pure, testable) ------------------

/// The additive `carryon_import_cut` outcome JSON.
public enum PhotoCraftImportOutcome: Sendable, Equatable {
    case completed(mirrorCut: UInt64)
    case partial(published: Int, deferred: Int, deferredBytes: UInt64)
    case suspended

    public static func parse(_ json: String) -> PhotoCraftImportOutcome? {
        guard let v = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else {
            return nil
        }
        if let done = v["completed"] as? Bool, done, let n = v["mirror_cut"] as? NSNumber {
            return .completed(mirrorCut: n.uint64Value)
        }
        if let partial = v["partial_budget"] as? Bool, partial {
            let published = (v["published"] as? [Any])?.count ?? 0
            let deferred = (v["deferred"] as? [Any])?.count ?? 0
            let bytes = (v["deferred_bytes"] as? NSNumber)?.uint64Value ?? 0
            return .partial(published: published, deferred: deferred, deferredBytes: bytes)
        }
        if let suspended = v["suspended"] as? Bool, suspended {
            return .suspended
        }
        return nil
    }
}

/// What the receiving UI shows while a carry streams in: real per-object
/// progress from the engine's §6.8 budget windows (the tiles).
public struct PhotoCraftProgress: Sendable, Equatable {
    public var totalObjects: Int
    public var publishedObjects: Int
    public var windows: Int
    public var wireBytes: UInt64
    public var windowBytes: UInt64
    public var widenedWindows: Int

    public init(totalObjects: Int, publishedObjects: Int, windows: Int,
                wireBytes: UInt64, windowBytes: UInt64, widenedWindows: Int) {
        self.totalObjects = totalObjects
        self.publishedObjects = publishedObjects
        self.windows = windows
        self.wireBytes = wireBytes
        self.windowBytes = windowBytes
        self.widenedWindows = widenedWindows
    }

    public var fraction: Double {
        guard totalObjects > 0 else { return 0 }
        return min(1.0, Double(publishedObjects) / Double(totalObjects))
    }
}

// ---- events + results --------------------------------------------------------

public enum PhotoCraftEvent: Sendable {
    case console(String)
    case live(PhotoCraftLiveSession)
    case capture(PhotoCraftCapture)
    /// The sender's listening address + its own pin (the receiver pins it).
    case listening(addr: String, pinHex: String)
    case descriptor(PhotoCraftDescriptor)
    case progress(PhotoCraftProgress)
    case served(connections: Int, bytesSent: UInt64)
    /// Single-writer authority as this side now knows it (#24).
    case authority(PhotoCraftAuthorityReport)
    case finished(PhotoCraftResult)
    case failed(String)
}

public struct PhotoCraftResult: Sendable {
    public var role: String        // "send" | "receive"
    public var label: String       // always OBSERVED (LOCAL) for this path
    public var name: String
    public var objects: Int
    public var windows: Int
    public var wireBytes: UInt64
    public var seconds: Double
    public var restore: PhotoCraftRestoreReport?
}

// ---- single-writer authority in the pane (L4, #24; pure, testable) -----------

/// Who may write the carried PhotoCraft session, as this pane knows it. Only
/// .origin and .thisInstance let Carry-On write; everything else fails closed.
public enum PhotoCraftAuthorityState: String, Sendable, Equatable, CaseIterable {
    /// No carry has touched authority here: a fresh local session is its own single writer.
    case origin
    /// This instance received authority by an explicit hand-over (engine may_mutate = true).
    case thisInstance
    /// The other instance holds it; this one is a read-only replica.
    case peer
    /// A hand-over is on the wire (offer, accept, receipts).
    case handingOver
    /// Both sides claim authority: two writers. Refused until §21.3 recovery.
    case conflict
    /// Nobody provably holds it (a hand-over died mid-way). Refused until §21.3 recovery.
    case ambiguous

    /// The holder line the pane shows.
    public var holder: String {
        switch self {
        case .origin: return "this instance (origin, single writer)"
        case .thisInstance: return "this instance (received by hand-over)"
        case .peer: return "the other instance (this one is a read-only replica)"
        case .handingOver: return "hand-over in flight"
        case .conflict: return "CONFLICT: both instances claim authority"
        case .ambiguous: return "AMBIGUOUS: no instance provably holds authority"
        }
    }

    /// Carry-On may write (seal a new cut of the live app) only as the single writer.
    public var mayWrite: Bool { self == .origin || self == .thisInstance }
}

/// What the pane is about to do.
public enum PhotoCraftWriteAttempt: String, Sendable, Equatable {
    /// Capture the live app and seal a new cut: a write by this instance.
    case send
    /// Receive a carried session as a read-only replica.
    case receive
    /// Receive, then take single-writer authority (explicit hand-over).
    case receiveAndTakeAuthority
}

public struct PhotoCraftAuthorityReport: Sendable, Equatable {
    public var state: PhotoCraftAuthorityState
    /// The engine's own answer (carryon_may_mutate) when the report was made.
    public var mayMutate: Bool
    /// The transfer receipt set JSON (new_epoch, receipts): data only, no secrets.
    public var receiptJSON: String?
    public var detail: String

    public init(state: PhotoCraftAuthorityState, mayMutate: Bool,
                receiptJSON: String? = nil, detail: String) {
        self.state = state
        self.mayMutate = mayMutate
        self.receiptJSON = receiptJSON
        self.detail = detail
    }
}

public enum PhotoCraftAuthority {
    /// The client feature that asks the source for a hand-over on this hop.
    public static let handOverRequest = "org.carryon.photocraft.ux.authority=request"

    /// A hop that asks for authority (and is not the final done-ack).
    public static func isHandOverRequest(features: [String]) -> Bool {
        features.contains(handOverRequest) && !PhotoCraftDescriptor.isDone(features: features)
    }

    /// Combine this core's may_mutate with what the transfer says about the peer.
    /// Two claims are a CONFLICT, no claim is AMBIGUOUS: both fail closed.
    public static func reconcile(localMayMutate: Bool, peerHolds: Bool) -> PhotoCraftAuthorityState {
        switch (localMayMutate, peerHolds) {
        case (true, true): return .conflict
        case (true, false): return .thisInstance
        case (false, true): return .peer
        case (false, false): return .ambiguous
        }
    }

    /// The pane's gate: nil when allowed, otherwise why it is refused (fail closed).
    public static func refusal(_ attempt: PhotoCraftWriteAttempt,
                               state: PhotoCraftAuthorityState) -> String? {
        switch state {
        case .conflict:
            return "two writers claim this session; refused until §21.3 recovery (fail closed)"
        case .ambiguous:
            return "no instance provably holds authority; refused until §21.3 recovery (fail closed)"
        case .handingOver:
            return "a hand-over is in flight; wait for its receipts"
        case .peer:
            return attempt == .send
                ? "the other instance holds authority; a new cut here would be a second writer (fail closed). Take it back with Receive + hand-over first."
                : nil
        case .origin, .thisInstance:
            return nil
        }
    }
}

// ---- the engine budget as JSON (all fields; §6.8) ----------------------------

public enum PhotoCraftBudget {
    public static func json(totalBytes: UInt64) -> String {
        "{\"total_net_bytes\":\(totalBytes),\"avg_net_rate\":18446744073709551615," +
        "\"burst_bytes\":18446744073709551615,\"total_cpu_ms\":60000,\"cpu_duty\":1.0," +
        "\"peak_prep_mem\":536870912,\"storage_quota\":18446744073709551615," +
        "\"battery_thermal\":\"Unrestricted\",\"elapsed_opportunity_ms\":60000," +
        "\"max_nonpreemptible_ms\":5000}"
    }
}

// ---- send / receive ----------------------------------------------------------

public struct PhotoCraftSendConfig: Sendable {
    public var live: PhotoCraftLiveConfig
    /// The RECEIVER's pin: the sender binds, the receiver dials it.
    public var receiverPin: String
    public var dataDir: String
    public var windowBytes: UInt64
    public var encoding: String
    public var identity: DeviceIdentity

    public init(live: PhotoCraftLiveConfig, receiverPin: String,
                dataDir: String, windowBytes: UInt64 = 256 * 1024,
                encoding: String = "split", identity: DeviceIdentity) {
        self.live = live
        self.receiverPin = receiverPin
        self.dataDir = dataDir
        self.windowBytes = windowBytes
        self.encoding = encoding
        self.identity = identity
    }
}

public struct PhotoCraftReceiveConfig: Sendable {
    public var live: PhotoCraftLiveConfig
    public var sourceAddr: String
    public var sourcePin: String
    /// Explicit opt-in (#24): after a verified restore, ask the source for
    /// single-writer authority. Off by default: receiving alone makes a replica.
    public var takeAuthority: Bool
    public var dataDir: String
    public var windowBytes: UInt64
    public var identity: DeviceIdentity

    public init(live: PhotoCraftLiveConfig, sourceAddr: String, sourcePin: String,
                dataDir: String, windowBytes: UInt64 = 256 * 1024,
                identity: DeviceIdentity, takeAuthority: Bool = false) {
        self.takeAuthority = takeAuthority
        self.live = live
        self.sourceAddr = sourceAddr
        self.sourcePin = sourcePin
        self.dataDir = dataDir
        self.windowBytes = windowBytes
        self.identity = identity
    }
}

/// Source side: capture the live session, seal the cut, serve the receiver's
/// budget windows until it acks. Blocking: call on `Task.detached`.
public enum PhotoCraftSender {
    public static func run(_ cfg: PhotoCraftSendConfig,
                           emit: @Sendable (PhotoCraftEvent) -> Void) throws -> PhotoCraftResult {
        let started = Date()
        emit(.console("PhotoCraft live: \(cfg.live.addr) (control channel, loopback)"))
        let live = try PhotoCraftBridge.inspect(cfg.live)
        emit(.live(live))
        emit(.console("live: \(live.name) · rev \(live.revision) · dirty \(live.dirty) · \(live.tool)"))

        try FileManager.default.createDirectory(atPath: cfg.dataDir,
                                                withIntermediateDirectories: true)
        let engine = try Engine.open(dataDir: cfg.dataDir)
        let capture = try PhotoCraftBridge.register(
            engine, cfg.live, encoding: cfg.encoding,
            sessionTitle: "PhotoCraft: \(live.name)")
        emit(.capture(capture))
        emit(.console("captured \(capture.encoding): \(capture.parts) parts · \(capture.documentBytes) B document · \(capture.unsavedBytes) B unsaved"))

        let session = try engine.createSession(
            adapterID: capture.adapterID, title: "PhotoCraft: \(capture.name)",
            privacy: "personal", authorityMode: "single_writer")
        let cut = try engine.createCut(sessionID: session)
        // meta + the (possibly empty) unsaved object + the document root + parts.
        // The adapter always declares the unsaved object, so it is counted even
        // when the document is clean (0 B).
        let objects = capture.parts + 3
        let descriptor = PhotoCraftDescriptor(
            session: session, cut: cut.number, objects: objects,
            name: capture.name, dirty: capture.dirty, workspaceJSON: capture.workspaceJSON)

        // Trust: this device pins the receiver; the receiver pins this device.
        let trust = try TrustStore.fromJSON("{\"peers\":{}}")
        try trust.pair(name: "photocraft-peer", pinHex: cfg.receiverPin,
                       utc: ISO8601DateFormatter().string(from: Date()))
        let listener = try NetListener.bind("127.0.0.1:0")
        let addr = try listener.addr()
        let pin = try cfg.identity.pinHex()
        emit(.listening(addr: addr, pinHex: pin))
        emit(.console("serving \(descriptor.objects) objects on \(addr); receiver pin \(cfg.receiverPin.prefix(12))…"))

        var connections = 0
        var sent: UInt64 = 0
        while connections < 256 {
            connections += 1
            guard let net = try? NetSession.accept(listener: listener, identity: cfg.identity, trust: trust)
            else { break }
            let theirs = (try? net.serverNegotiate(features: descriptor.features)) ?? []
            if PhotoCraftAuthority.isHandOverRequest(features: theirs) {
                // Explicit hand-over (#24): the receiver asked for single-writer
                // authority after its verified restore. BLOCKING until it accepts.
                emit(.authority(PhotoCraftAuthorityReport(
                    state: .handingOver, mayMutate: true,
                    detail: "the receiver asked for authority; offering it (§21.2)")))
                do {
                    let receipts = try engine.serveAuthorityTransfer(
                        session: net, sessionID: session, cutNumber: cut.number)
                    // An unreadable may_mutate counts as "still a writer": with the
                    // peer now holding it, that reconciles to CONFLICT (fail closed).
                    let local = (try? engine.mayMutate(sessionID: session)) ?? true
                    emit(.authority(PhotoCraftAuthorityReport(
                        state: PhotoCraftAuthority.reconcile(localMayMutate: local, peerHolds: true),
                        mayMutate: local, receiptJSON: receipts,
                        detail: local
                            ? "the transfer returned but this core still may mutate"
                            : "handed over: this instance is a read-only replica now")))
                } catch {
                    // Which side committed is unknown here: fail closed.
                    let local = (try? engine.mayMutate(sessionID: session)) ?? false
                    emit(.authority(PhotoCraftAuthorityReport(
                        state: .ambiguous, mayMutate: local,
                        detail: "hand-over failed: \(error.localizedDescription)")))
                    emit(.console("authority hand-over failed: \(error.localizedDescription)"))
                }
                if let c = try? net.byteCounters() { sent += c.sent }
                net.teardown()
                continue
            }
            if PhotoCraftDescriptor.isDone(features: theirs) {
                if let c = try? net.byteCounters() { sent += c.sent }
                net.teardown()
                break
            }
            // A window closes with a BUDGET_PARTIAL Abort — normal, not a failure.
            _ = try? engine.serveCut(session: net)
            if let c = try? net.byteCounters() { sent += c.sent }
            net.teardown()
            emit(.console("served window \(connections) (\(sent) B sent)"))
        }
        engine.close()
        emit(.served(connections: connections, bytesSent: sent))
        let result = PhotoCraftResult(
            role: "send", label: "OBSERVED (LOCAL)", name: capture.name,
            objects: descriptor.objects, windows: connections, wireBytes: sent,
            seconds: Date().timeIntervalSince(started), restore: nil)
        emit(.finished(result))
        return result
    }
}

/// Destination side: read the descriptor, import in §6.8 windows (widening when
/// an object is larger than the window — the engine's fail-closed rule), restore
/// into the local live app, and acknowledge.
public enum PhotoCraftReceiver {
    public static func run(_ cfg: PhotoCraftReceiveConfig,
                           emit: @Sendable (PhotoCraftEvent) -> Void) throws -> PhotoCraftResult {
        let started = Date()
        try FileManager.default.createDirectory(atPath: cfg.dataDir,
                                                withIntermediateDirectories: true)
        let engine = try Engine.open(dataDir: cfg.dataDir)
        try engine.setChunkSize(64 * 1024)
        let trust = try TrustStore.fromJSON("{\"peers\":{}}")
        try trust.pair(name: "photocraft-source", pinHex: cfg.sourcePin,
                       utc: ISO8601DateFormatter().string(from: Date()))

        // First hop: the source's Welcome features carry the descriptor.
        let first = try NetSession.connect(addr: cfg.sourceAddr, identity: cfg.identity, trust: trust)
        let features = try first.clientNegotiate(features: [PhotoCraftDescriptor.marker])
        guard let descriptor = PhotoCraftDescriptor.parse(features: features) else {
            first.teardown()
            throw CarryonError(code: -4, message: "source is not offering a PhotoCraft session")
        }
        emit(.descriptor(descriptor))
        emit(.console("incoming \(descriptor.name) · \(descriptor.objects) objects · cut \(descriptor.cut)"))

        var windows = 0
        var widened = 0
        var published = 0
        var wire: UInt64 = 0
        var window = cfg.windowBytes
        var mirrorCut: UInt64 = 0
        var net = first
        var haveHop = true
        var completed = false
        while windows < 512 && !completed {
            windows += 1
            if !haveHop {
                net = try NetSession.connect(addr: cfg.sourceAddr, identity: cfg.identity, trust: trust)
                _ = try net.clientNegotiate(features: [PhotoCraftDescriptor.marker])
            }
            haveHop = false
            try engine.setBudget(json: PhotoCraftBudget.json(totalBytes: window))
            do {
                let json = try engine.importCut(session: net, remoteSession: descriptor.session,
                                                remoteCut: descriptor.cut)
                if let c = try? net.byteCounters() { wire += c.recv }
                net.teardown()
                switch PhotoCraftImportOutcome.parse(json) {
                case .completed(let n):
                    mirrorCut = n
                    published = descriptor.objects
                    completed = true
                case .partial(let pub, let deferred, _):
                    published += pub
                    emit(.progress(PhotoCraftProgress(
                        totalObjects: descriptor.objects, publishedObjects: published,
                        windows: windows, wireBytes: wire, windowBytes: window,
                        widenedWindows: widened)))
                    emit(.console("window \(windows): +\(pub) objects (\(deferred) deferred), \(wire) B"))
                    window = cfg.windowBytes // every window is its own budget
                case .suspended:
                    throw CarryonError(code: -4, message: "unexpected suspend")
                case nil:
                    throw CarryonError(code: -4, message: "unparseable import outcome")
                }
            } catch let e as CarryonError where e.name.hasPrefix("BUDGET") {
                // One object is larger than the window: widen and retry (the
                // engine refuses fail-closed; the UI shows the widening).
                net.teardown()
                widened += 1
                window = min(window * 2, 512 * 1024 * 1024)
                windows -= 1 // a refused attempt is not a progress window
                emit(.console("window refused by the engine (\(e.name)); widening to \(window) B"))
                continue
            }
            haveHop = false
        }
        guard completed else {
            engine.close()
            throw CarryonError(code: -4, message: "carry did not complete within 512 windows")
        }
        emit(.progress(PhotoCraftProgress(
            totalObjects: descriptor.objects, publishedObjects: descriptor.objects,
            windows: windows, wireBytes: wire, windowBytes: window, widenedWindows: widened)))

        // Restore into the LOCAL live PhotoCraft. The ephemeral working position
        // travels in the descriptor (data only); the document, edits and meta are
        // the engine's sealed objects.
        let mirror = try engine.mirrorSessionID(remoteSession: descriptor.session)
        let restore = try PhotoCraftBridge.restore(
            engine, cfg.live, workspaceJSON: descriptor.workspaceJSON,
            nameHint: descriptor.name, mirrorSession: mirror, mirrorCut: mirrorCut)
        emit(.console("restored into live PhotoCraft: \(restore.restored.name) · tool \(restore.restored.tool) · zoom \(restore.restored.zoom) · oracle agreed \(restore.oracle.agreed)"))
        guard restore.oracle.checked && restore.oracle.agreed else {
            engine.close()
            throw CarryonError(code: -4, message: "restore oracle disagreed: \(restore.oracle.detail)")
        }

        // Authority (#24). Only an explicit opt-in asks for a hand-over; the mirror
        // must not already claim authority (that would be a second writer, so it
        // is refused before anything moves). Without the opt-in this side is a
        // read-only replica and the source stays the single writer.
        if cfg.takeAuthority {
            let before = (try? engine.mayMutate(sessionID: mirror)) ?? true
            if before {
                emit(.authority(PhotoCraftAuthorityReport(
                    state: .conflict, mayMutate: true,
                    detail: "the mirror already claims authority; refusing a second writer")))
                engine.close()
                throw CarryonError(code: -4, message: "two writers: the mirror already claims authority (fail closed)")
            }
            emit(.authority(PhotoCraftAuthorityReport(
                state: .handingOver, mayMutate: false,
                detail: "asking the source for single-writer authority (§21.2)")))
            do {
                let hop = try NetSession.connect(addr: cfg.sourceAddr, identity: cfg.identity, trust: trust)
                defer { hop.teardown() }
                _ = try hop.clientNegotiate(features: [PhotoCraftDescriptor.marker,
                                                       PhotoCraftAuthority.handOverRequest])
                let receipts = try engine.requestAuthorityTransfer(
                    session: hop, mirrorSessionID: mirror, adapterID: restore.adapterID)
                let after = (try? engine.mayMutate(sessionID: mirror)) ?? false
                emit(.authority(PhotoCraftAuthorityReport(
                    state: PhotoCraftAuthority.reconcile(localMayMutate: after, peerHolds: false),
                    mayMutate: after, receiptJSON: receipts,
                    detail: after
                        ? "this instance holds single-writer authority now"
                        : "the transfer returned but this core may not mutate")))
                emit(.console("authority: taken over (may_mutate \(after))"))
            } catch {
                let local = (try? engine.mayMutate(sessionID: mirror)) ?? false
                emit(.authority(PhotoCraftAuthorityReport(
                    state: .ambiguous, mayMutate: local,
                    detail: "hand-over failed: \(error.localizedDescription)")))
                emit(.console("authority hand-over failed: \(error.localizedDescription)"))
            }
        } else {
            let local = (try? engine.mayMutate(sessionID: mirror)) ?? true
            emit(.authority(PhotoCraftAuthorityReport(
                state: PhotoCraftAuthority.reconcile(localMayMutate: local, peerHolds: true),
                mayMutate: local,
                detail: local
                    ? "the mirror claims authority without a hand-over"
                    : "read-only replica: no hand-over was requested; the source stays the single writer")))
        }

        // Acknowledge: one last hop so the sender stops serving.
        if let ack = try? NetSession.connect(addr: cfg.sourceAddr, identity: cfg.identity, trust: trust) {
            _ = try? ack.clientNegotiate(features: [PhotoCraftDescriptor.marker,
                                                    PhotoCraftDescriptor.done])
            ack.teardown()
        }
        engine.close()
        let result = PhotoCraftResult(
            role: "receive", label: "OBSERVED (LOCAL)", name: descriptor.name,
            objects: descriptor.objects, windows: windows, wireBytes: wire,
            seconds: Date().timeIntervalSince(started), restore: restore)
        emit(.finished(result))
        return result
    }
}
