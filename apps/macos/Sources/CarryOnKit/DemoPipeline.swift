/*
 * DemoPipeline.swift — the 8-stage demo pipeline from plan.md, driving the REAL
 * engine through the C ABI: two independent cores in temp dirs on the same
 * machine, real TLS 1.3 loopback, mutual pin pairing, L4 authority transfer.
 *
 * Claim label (§2 law): this is OBSERVED (LOCAL) — one device, loopback TLS;
 * NOT physical cross-device evidence, and not a supported platform (§30 unrun).
 *
 * Every blocking FFI call (serve_cut, import_cut, serve/request_authority_transfer,
 * TLS accept/connect, negotiation) runs on Task.detached off the caller — the
 * pipeline actor never blocks. `carryon_session_bytes` is NEVER read while a
 * blocking wire op is in flight on the same session: the C ABI guarantees no
 * thread-safety for a CarryonSession, so counters are sampled only BETWEEN
 * completed operations (sampleCounters below).
 */

import CCarryon
import Foundation

/// Exact §16.2 session states — never the bare word "Ready" (plan.md law).
public enum SessionState: String, CaseIterable, Sendable {
    case idle = "Idle"
    case preparing = "Preparing"
    case cutProposed = "CutProposed"
    case cutSealed = "CutSealed"
    case importing = "Importing"
    case actionReady = "ActionReady"
    case readOnlyActive = "ReadOnlyActive"
    case failed = "Failed"
    case inconclusive = "Inconclusive"
    case aborted = "Aborted"
}

public enum DemoSide: String, Sendable { case source, destination }

/// The 8 pipeline stages (plan.md §2).
public enum DemoStage: Int, CaseIterable, Sendable, Identifiable {
    case open = 1, identity, pair, session, transfer, authority, edit, evidence

    public var id: Int { rawValue }
    public var label: String {
        switch self {
        case .open: return "OPEN"
        case .identity: return "IDENTITY"
        case .pair: return "PAIR"
        case .session: return "SESSION"
        case .transfer: return "TRANSFER"
        case .authority: return "AUTHORITY"
        case .edit: return "EDIT"
        case .evidence: return "EVIDENCE"
        }
    }
}

/// Scenario switch (plan.md §2): the L3 editor flagship or the graph handoff.
public enum DemoScenario: String, CaseIterable, Sendable, Identifiable {
    case editorL3
    case graph

    public var id: String { rawValue }

    public var adapterID: String {
        switch self {
        case .editorL3: return "org.carryon.editor"
        case .graph: return "org.carryon.graph"
        }
    }

    /// Adapter params JSON per the compiled-in factory (crates/carryon-ffi/src/factory.rs).
    public var adapterParamsJSON: String {
        switch self {
        case .editorL3: return "{\"session_v1\":true}"
        case .graph: return "{\"sample\":true}"
        }
    }

    public var title: String {
        switch self {
        case .editorL3: return "L3 editor handoff (flagship)"
        case .graph: return "Graph handoff"
        }
    }

    /// The destination's continuation action (stage 7).
    public var editClass: String {
        switch self {
        case .editorL3: return "document.edit"
        case .graph: return "graph.shortest_path"
        }
    }

    public var editParams: AnyJSON {
        switch self {
        case .editorL3:
            return .object(["text": .string("continued on the macOS destination after authority moved [carry-on-macos]")])
        case .graph:
            return .object(["start": .number(0), "end": .number(4)])
        }
    }

    /// org.carryon.graph declares supports_authority_transfer:false (L4 is the
    /// editor flagship) — the AUTHORITY stage is skipped honestly for this scenario.
    public var supportsAuthorityTransfer: Bool { self == .editorL3 }
}

/// Demo knobs (plan.md §2 "Advanced showcase toggles").
public struct DemoConfig: Sendable {
    public var scenario: DemoScenario = .editorL3
    /// Destination chunk size in bytes (§18.6 progressive transfer); 0 = engine default.
    public var chunkSize: UInt64 = 0
    /// Tiny net budget on the destination → partial admission shown as a NORMAL
    /// outcome (§6.8: budget rejection SHALL be visible), then replenished + retry.
    public var budgetRejection = false
    /// Pre-arm `carryon_core_request_suspend` (§11.2 allows arming before or during
    /// an import), observe the suspended outcome, then `carryon_resume_import`.
    public var suspendResume = false
    /// Root for the two temp data dirs + the evidence file (default: fresh temp dir).
    public var workRoot: String? = nil

    public init() {}
}

public enum StageOutcome: Sendable, Equatable {
    case ok
    case skipped(String)
    case failed(String)
}

/// One console row; rendered repo-style as `key:  value` (plan.md ASCII style).
public struct ConsoleLine: Sendable, Equatable, Identifiable {
    public var indent: Int
    public var key: String
    public var value: String
    public var id: Int

    public init(indent: Int = 0, key: String = "", value: String) {
        self.indent = indent
        self.key = key
        self.value = value
        self.id = ConsoleLine.nextID()
    }

    private static let counter = ManagedAtomicLazyBox()
    public var rendered: String {
        let pad = String(repeating: "    ", count: indent)
        if key.isEmpty { return pad + value }
        return pad + key + ":" + String(repeating: " ", count: max(2, 14 - key.count)) + value
    }

    private static func nextID() -> Int {
        counter.advance()
    }
}

/// Tiny thread-safe counter so ConsoleLine conforms to Identifiable for SwiftUI.
final class ManagedAtomicLazyBox: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    func advance() -> Int {
        lock.lock(); defer { lock.unlock() }
        n += 1
        return n
    }
}

/// Live events emitted during a run.
public enum DemoEvent: Sendable {
    case console(ConsoleLine)
    case stageStarted(DemoStage)
    case stageFinished(DemoStage, StageOutcome)
    case counters(sent: UInt64, recv: UInt64)
    case mutationFlipped(side: DemoSide, session: String, may: Bool)
    case sessionState(side: DemoSide, session: String, state: SessionState)
    case finished(DemoResult)
    case failed(String)
}

/// Everything a run produced (for the UI + the smoke test).
public struct DemoResult: Sendable {
    public var ok: Bool
    public var scenario: DemoScenario
    public var workRoot: String
    public var evidencePath: String
    public var bundleJSON: String
    public var bundle: EvidenceBundle?
    public var verifyOK: Bool
    public var verifyMessage: String
    public var sourceSessionID: String
    public var mirrorSessionID: String
    public var mirrorCut: UInt64
    public var newEpoch: UInt64?
    public var sourcePin: String
    public var destPin: String
    public var bytesSent: UInt64
    public var bytesRecv: UInt64
    public var mayMutateDestination: Bool
    public var mayMutateSource: Bool
    /// Serialized TrustStore halves from the pairing (pins only; no secrets).
    public var localTrustJSON: String
    public var remoteTrustJSON: String
    public var sourceTimeline: [SessionState]
    public var destTimeline: [SessionState]
    public var stageOutcomes: [DemoStage: StageOutcome]
    public var console: [ConsoleLine]
    public var elapsedMS: Double
    public var failure: String?

    /// The fixed footer, verbatim from plan.md.
    public static let disclosureLines: [String] = [
        "DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and",
        "engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30).",
    ]
}

/// The verbatim UI label for demo mode (plan.md §2).
public let demoModeLabel =
    "OBSERVED (LOCAL) — one device, loopback TLS 1.3; not physical cross-device evidence (spec §2/§30)"

/// Import outcome JSON shapes from the ABI (write_outcome in abi_transfer.rs),
/// plus the two budget-refusal presentations (§6.8): a partial admission JSON,
/// or a typed refusal (code 500, first object over budget) before any bytes.
enum ImportOutcome {
    case completed(mirrorCut: UInt64)
    case suspended(resumeTokenJSON: String)
    case partialBudget
    case budgetRefused(CarryonError)
    case other(raw: String)

    static func parse(_ json: String) -> ImportOutcome {
        guard let v = parseAnyJSON(json), case .object(let o) = v else { return .other(raw: json) }
        if o["completed"]?.boolValue == true {
            let cut = o["mirror_cut"]?.numberValue.map { UInt64($0) } ?? 0
            return .completed(mirrorCut: cut)
        }
        if o["suspended"]?.boolValue == true {
            let token = o["resume_token"]?.jsonEncoded ?? "{}"
            return .suspended(resumeTokenJSON: token)
        }
        if o["partial_budget"]?.boolValue == true { return .partialBudget }
        return .other(raw: json)
    }
}

func nowMs() -> Double {
    Double(DispatchTime.now().uptimeNanoseconds) / 1_000_000.0
}

func nowUTC() -> String {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime]
    return f.string(from: Date())
}

/// Run a blocking FFI body on a detached task (never on the caller/actor).
/// Handles are @unchecked Sendable wrappers over raw C handles; each handle is
/// touched by one flow at a time (request_suspend is documented thread-safe).
@discardableResult
func det<T: Sendable>(_ what: String, _ body: @escaping @Sendable () throws -> T) async throws -> T {
    let _ = what // names aid debugger backtraces in spirit; kept simple
    return try await Task.detached(priority: .userInitiated) {
        try body()
    }.value
}

// Budget JSONs (§6.8; serde requires every field of model::Budget). The tiny
// budget admits a strict PREFIX of the closure (each demo object is < 208 bytes,
// so 128 forces a visible partial admission; §6.8).
let tinyBudgetBytes: UInt64 = 128
let tinyBudgetJSON =
    "{\"total_net_bytes\":128,\"avg_net_rate\":65536,\"burst_bytes\":1024,\"total_cpu_ms\":60000," +
    "\"cpu_duty\":1.0,\"peak_prep_mem\":67108864,\"storage_quota\":1073741824," +
    "\"battery_thermal\":\"Unrestricted\",\"elapsed_opportunity_ms\":60000,\"max_nonpreemptible_ms\":5000}"
let generousBudgetJSON =
    "{\"total_net_bytes\":18446744073709551615,\"avg_net_rate\":18446744073709551615," +
    "\"burst_bytes\":18446744073709551615,\"total_cpu_ms\":60000,\"cpu_duty\":1.0," +
    "\"peak_prep_mem\":536870912,\"storage_quota\":18446744073709551615," +
    "\"battery_thermal\":\"Unrestricted\",\"elapsed_opportunity_ms\":60000,\"max_nonpreemptible_ms\":5000}"

public actor DemoPipeline {
    public private(set) var lines: [ConsoleLine] = []

    public init() {}

    /// Run the full 8-stage pipeline. `emit` receives progress events; the
    /// returned result carries the console, the bundle, and the verify verdict.
    @discardableResult
    public func run(_ config: DemoConfig = DemoConfig(),
                    emit: @escaping @Sendable (DemoEvent) -> Void = { _ in }) async throws -> DemoResult {
        lines = []
        let t0 = nowMs()

        func log(_ key: String, _ value: String, indent: Int = 1) {
            let line = ConsoleLine(indent: indent, key: key, value: value)
            lines.append(line)
            emit(.console(line))
        }
        func logPlain(_ value: String) {
            let line = ConsoleLine(indent: 0, value: value)
            lines.append(line)
            emit(.console(line))
        }
        func logBlank() {
            let line = ConsoleLine(indent: 0, value: "")
            lines.append(line)
            emit(.console(line))
        }
        func begin(_ stage: DemoStage) {
            emit(.stageStarted(stage))
            logPlain("stage \(stage.rawValue)/8:  \(stage.label)")
        }
        func end(_ stage: DemoStage, _ outcome: StageOutcome) {
            emit(.stageFinished(stage, outcome))
        }
        func setState(_ side: DemoSide, _ session: String, _ state: SessionState) {
            emit(.sessionState(side: side, session: session, state: state))
        }
        func mutation(_ side: DemoSide, _ session: String, _ may: Bool) {
            emit(.mutationFlipped(side: side, session: session, may: may))
        }

        // ---- header -----------------------------------------------------
        logPlain("== Carry-On macOS demo — \(config.scenario.title) ==")
        log("abi", AbiVersion.current().description)
        log("binding", "CarryOnKit (Swift over C ABI 1.2)")
        log("label", demoModeLabel)

        // ---- 1. OPEN ----------------------------------------------------
        emit(.stageStarted(.open))
        logPlain("stage 1/8:  OPEN")
        try Task.checkCancellation()
        let root = config.workRoot
            ?? (NSTemporaryDirectory() as NSString)
                .appendingPathComponent("carryon-macos-demo-\(Int(Date().timeIntervalSince1970))")
        let srcDir = root + "/source"
        let dstDir = root + "/dest"
        try FileManager.default.createDirectory(atPath: srcDir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(atPath: dstDir, withIntermediateDirectories: true)

        let sourceEngine = try await det("core_open") { try Engine.open(dataDir: srcDir) }
        let destEngine = try await det("core_open") { try Engine.open(dataDir: dstDir) }
        let srcRec = try sourceEngine.recoveryReport()
        let dstRec = try destEngine.recoveryReport()
        log("source_dir", srcDir)
        log("dest_dir", dstDir)
        log("recovery_source", "clean=\(srcRec.clean) interrupted=\(srcRec.interrupted_transfers.count) hidden=\(srcRec.hidden_incomplete_objects.count) ambiguous=\(srcRec.ambiguous_sessions.count) truncated=\(srcRec.journal_truncated)")
        log("recovery_dest", "clean=\(dstRec.clean) interrupted=\(dstRec.interrupted_transfers.count) hidden=\(dstRec.hidden_incomplete_objects.count) ambiguous=\(dstRec.ambiguous_sessions.count) truncated=\(dstRec.journal_truncated)")
        emit(.stageFinished(.open, .ok))

        // ---- 2. IDENTITY --------------------------------------------------
        emit(.stageStarted(.identity))
        logPlain("stage 2/8:  IDENTITY")
        let sourceID = try await det("identity_generate") { try DeviceIdentity.generate(name: "carry-on-macos-source") }
        let destID = try await det("identity_generate") { try DeviceIdentity.generate(name: "carry-on-macos-dest") }
        let sourcePin = try sourceID.pinHex()
        let destPin = try destID.pinHex()
        log("source_pin", "\(sourcePin.prefix(12))…  (truncated; pairing fingerprint)")
        log("dest_pin", "\(destPin.prefix(12))…  (truncated; pairing fingerprint)")
        log("key_der", "<never logged or displayed — SECRET (EVD-006/§22.3)>")
        emit(.stageFinished(.identity, .ok))

        // ---- 3. PAIR ------------------------------------------------------
        emit(.stageStarted(.pair))
        logPlain("stage 3/8:  PAIR")
        let pairing = try await det("pair_devices") { try pairDevices(sourceID, destID, utc: nowUTC()) }
        // Round-trip each half through carryon_trust_from_json (engine-native form).
        let sourceTrust = try TrustStore.fromJSON(pairing.localTrustJSON)
        let destTrust = try TrustStore.fromJSON(pairing.remoteTrustJSON)
        log("pairing", "mutual pin (SHA-256 of cert DER), utc=\(nowUTC())")
        log("source_trust_records", "\(try sourceTrust.records().count)")
        log("dest_trust_records", "\(try destTrust.records().count)")
        emit(.stageFinished(.pair, .ok))

        // ---- 4. SESSION ---------------------------------------------------
        emit(.stageStarted(.session))
        logPlain("stage 4/8:  SESSION")
        try await det("register_adapter") {
            try sourceEngine.registerAdapter(id: config.scenario.adapterID,
                                             paramsJSON: config.scenario.adapterParamsJSON)
        }
        try await det("register_adapter") {
            try destEngine.registerAdapter(id: config.scenario.adapterID,
                                           paramsJSON: config.scenario.adapterParamsJSON)
        }
        log("adapter", "\(config.scenario.adapterID)  params \(config.scenario.adapterParamsJSON)")
        let sourceSession = try await det("create_session") {
            try sourceEngine.createSession(adapterID: config.scenario.adapterID,
                                           title: "carry-on-macos demo",
                                           privacy: "personal",
                                           authorityMode: "single_writer")
        }
        // The mirror id is deterministic (§16): usable as the dest key right away.
        let mirrorSession = try await det("mirror_session_id") {
            try CoreStatic.mirrorSessionID(remoteSession: sourceSession)
        }
        let cut = try await det("create_cut") { try sourceEngine.createCut(sessionID: sourceSession) }
        emit(.sessionState(side: .source, session: sourceSession, state: .preparing))
        emit(.sessionState(side: .source, session: sourceSession, state: .cutProposed))
        emit(.sessionState(side: .source, session: sourceSession, state: .cutSealed))
        emit(.sessionState(side: .destination, session: mirrorSession, state: .idle))
        log("source_session", "\(sourceSession)  authority_mode=single_writer")
        log("source_cut", "{session: \(cut.session), number: \(cut.number)}")
        emit(.stageFinished(.session, .ok))

        // ---- 5. TRANSFER --------------------------------------------------
        emit(.stageStarted(.transfer))
        logPlain("stage 5/8:  TRANSFER")
        emit(.sessionState(side: .destination, session: mirrorSession, state: .preparing))

        // One TLS hop: bind → connect (client first, it needs the server) →
        // accept → negotiate. A partial-budget retry opens a FRESH hop (the
        // engine closes the wire with Abort on a partial; transfer.rs).
        var allSessions: [NetSession] = []
        func openTransport() async throws -> (listener: NetListener, src: NetSession, dst: NetSession) {
            let l = try await det("listener_bind") { try NetListener.bind("127.0.0.1:0") }
            let addr = try l.addr()
            log("listener", addr)
            log("tls", "1.3, mutual certificate pinning (§18.5)")
            // The client handshake needs the server: run connect + accept as a
            // task-group pair (AGENTS.md loopback note).
            var client: NetSession?
            var server: NetSession?
            try await withThrowingTaskGroup(of: (String, NetSession).self) { group in
                group.addTask { ("connect", try NetSession.connect(addr: addr, identity: destID, trust: destTrust)) }
                group.addTask { ("accept", try NetSession.accept(listener: l, identity: sourceID, trust: sourceTrust)) }
                for try await (role, session) in group {
                    if role == "connect" { client = session } else { server = session }
                }
            }
            guard let d = client, let s = server else {
                throw CarryonError(code: -2, message: "transport pair did not form")
            }
            // Negotiation is a Hello/Welcome pair — run both halves concurrently.
            async let clientFeatures = det("client_negotiate") { try d.clientNegotiate(features: []) }
            let serverFeatures = try await det("server_negotiate") { try s.serverNegotiate(features: []) }
            _ = try await clientFeatures
            log("negotiated_features", "[\(serverFeatures.joined(separator: ","))]")
            allSessions.append(s)
            allSessions.append(d)
            return (l, s, d)
        }

        var (_, srcSession, destSession) = try await openTransport()

        if config.chunkSize > 0 {
            // The engine admits chunk sizes in [4096, 8388608] (§18.6); clamp
            // the demo knob to the admitted range instead of failing the run.
            let chunk = min(max(config.chunkSize, 4096), 8_388_608)
            try destEngine.setChunkSize(chunk)
            log("chunk_size", "\(chunk)  (destination-controlled, §18.6)")
        }
        if config.budgetRejection {
            try destEngine.setBudget(json: tinyBudgetJSON)
            log("budget", "total_net_bytes=\(tinyBudgetBytes)  (tiny, §6.8 showcase)")
        }
        if config.suspendResume {
            // §11.2: the flag may be armed before or during the import; it is
            // cleared only when a resume begins. Arming first makes the demo
            // deterministic on a fast loopback.
            try destEngine.requestSuspend()
            log("suspend", "request_suspend armed (§11.2; cleared on resume)")
        }

        // Counter discipline (review P1): a CarryonSession is NOT thread-safe
        // (handle.rs as_mut contract: "not aliased elsewhere for the duration of
        // the borrow"). A detached poller's in-flight byteCounters would race the
        // blocking wire call itself (concurrent aliasing), and a cancelled-but-
        // executing read against a freed handle would be a use-after-free. So
        // there is NO poller: counters are sampled only BETWEEN completed wire
        // operations, when the actor is provably the sole toucher of the handle.
        func sampleCounters() {
            if let (s, r) = try? destSession.byteCounters() {
                emit(.counters(sent: s, recv: r))
            }
        }
        do {
            emit(.sessionState(side: .destination, session: mirrorSession, state: .importing))
            let serveSrc0 = srcSession // snapshot: detached tasks capture lets, not vars
            var serveTask = Task.detached(priority: .userInitiated) {
                try sourceEngine.serveCut(session: serveSrc0)
            }

            let tImportStart = nowMs()
            var outcome = try await attemptImport(destSession)
            sampleCounters() // between ops only — never during a blocking call
            var attempts = 1

            // Completion loop: suspended → resume (§11.2); partial budget →
            // replenish + FRESH connection + retry (published objects bank via
            // store dedup, §6.8/§6.6). The engine closes the wire with Abort on a
            // partial, so the source's serve_cut has returned by then.
            // An import attempt that is REFUSED by the budget before any bytes move
            // (code 500, first object over budget) is also a NORMAL §6.8 outcome —
            // shown, then the budget is replenished and the import retried on the
            // SAME connection (the source's serve loop never stopped).
            func attemptImport(_ session: NetSession) async throws -> ImportOutcome {
                do {
                    let json = try await det("import_cut") {
                        try destEngine.importCut(session: session, remoteSession: sourceSession, remoteCut: cut.number)
                    }
                    return ImportOutcome.parse(json)
                } catch let e as CarryonError where e.code == 500 && config.budgetRejection {
                    return .budgetRefused(e)
                }
            }
            importLoop: while true {
                if attempts > 6 {
                    throw CarryonError(code: 400, message: "import did not settle after \(attempts) attempts")
                }
                switch outcome {
                case .completed(let mirrorCut):
                    log("import_outcome", "{\"completed\":true,\"mirror_cut\":\(mirrorCut)}")
                    break importLoop
                case .suspended(let token):
                    log("import_outcome", "{\"suspended\":true}  resume_token=\(token.prefix(72))…")
                    log("resume", "carryon_resume_import (attempt \(attempts + 1))")
                    let ds = destSession // snapshot for the detached call
                    let resumeJSON = try await det("resume_import") {
                        try destEngine.resumeImport(session: ds, resumeTokenJSON: token)
                    }
                    outcome = ImportOutcome.parse(resumeJSON)
                    sampleCounters()
                    attempts += 1
                    continue
                case .budgetRefused(let e):
                    log("import_outcome", "refused before any bytes: \(e.description)  (fail closed, §6.8)")
                    log("budget", "replenished → retry (same connection; published nothing yet)")
                    try destEngine.setBudget(json: generousBudgetJSON)
                    outcome = try await attemptImport(destSession)
                    sampleCounters()
                    attempts += 1
                    continue
                case .partialBudget:
                    // Budget rejection is a NORMAL, visible outcome (§6.8): fail closed —
                    // nothing sealed until the closure completes; published objects keep.
                    log("import_outcome", "partial_budget=true  (budget admission visible, §6.8; fail closed)")
                    log("budget", "replenished → retry on a fresh connection (published objects skipped by dedup)")
                    try destEngine.setBudget(json: generousBudgetJSON)
                    // The wire closed with BUDGET_PARTIAL Abort: absorb the source's
                    // serve_cut result, then open a fresh TLS hop for the retry.
                    _ = try? await serveTask.value
                    (_, srcSession, destSession) = try await openTransport()
                    let serveSrcN = srcSession // snapshot for the detached call
                    serveTask = Task.detached(priority: .userInitiated) {
                        try sourceEngine.serveCut(session: serveSrcN)
                    }
                    outcome = try await attemptImport(destSession)
                    sampleCounters()
                    attempts += 1
                    continue
                case .other(let raw):
                    throw CarryonError(code: 400, message: "unexpected import outcome: \(raw)")
                }
            }
            guard case .completed(let mirrorCut) = outcome else {
                throw CarryonError(code: 400, message: "import never completed: \(outcome)")
            }
            _ = try await serveTask.value // serve_cut returns on ImportComplete

            let bytesAfterImport = try destSession.byteCounters()
            emit(.counters(sent: bytesAfterImport.sent, recv: bytesAfterImport.recv))
            let timeToActionReadyMs = nowMs() - tImportStart
            log("mirror_session", "\(mirrorSession)  mirror_cut=\(mirrorCut)")
            log("bytes", "sent=\(bytesAfterImport.sent) recv=\(bytesAfterImport.recv)")
            emit(.sessionState(side: .destination, session: mirrorSession, state: .actionReady))

            // Prove the SAME logical session restored (editor scenario, per carryon_l3.c).
            // restore_latency_ms is the MEASURED duration of session.restore itself
            // (review P2: it used to reuse timeToActionReady — the import
            // duration, not the restore op). 0.0 when the op never runs.
            var restoreAgreed = false
            var restoreLatencyMs: Double = 0
            if config.scenario == .editorL3 {
                let tRestoreStart = nowMs()
                let restore = try await det("session.restore") {
                    try destEngine.executeActionAs(
                        cut: CutRef(session: mirrorSession, number: mirrorCut),
                        class: "session.restore", params: .object([:]),
                        adapterID: DemoScenario.editorL3.adapterID)
                }
                restoreLatencyMs = nowMs() - tRestoreStart
                restoreAgreed = restore.oracleAgreed
                log("session.restore", "oracle agreed=\(restoreAgreed)  (\(String(format: "%.1f", restoreLatencyMs)) ms)")
            }
            emit(.stageFinished(.transfer, .ok))

            // ---- 6. AUTHORITY -------------------------------------------------
            emit(.stageStarted(.authority))
            logPlain("stage 6/8:  AUTHORITY")
            var newEpoch: UInt64?
            var mayMutateDest = false
            var mayMutateSource = true
            var authorityOutcome = StageOutcome.ok
            if config.scenario.supportsAuthorityTransfer {
                let pre = try destEngine.mayMutate(sessionID: mirrorSession)
                log("may_mutate(dest) pre-transfer", "\(pre)")
                emit(.mutationFlipped(side: .destination, session: mirrorSession, may: pre))
                if pre {
                    // Mirror must not start authoritative (§21.2) — enforce, don't
                    // just log (mirrors Android DemoRunner.kt:594-598, E_AUTH_BASE+2).
                    throw CarryonError(code: CARRYON_E_AUTH_BASE + 2,
                                       message: "mirror must not start authoritative")
                }

                // serve_authority_transfer (source) ⇄ request_authority_transfer
                // (dest): both block, so they run as a pair on detached tasks.
                // Snapshot the sessions: detached tasks capture lets, not vars.
                let authSrc = srcSession
                let authDst = destSession
                async let serveReceipt: String = det("serve_authority_transfer") {
                    try sourceEngine.serveAuthorityTransfer(session: authSrc,
                                                            sessionID: sourceSession,
                                                            cutNumber: cut.number)
                }
                let requestReceipt = try await det("request_authority_transfer") {
                    try destEngine.requestAuthorityTransfer(session: authDst,
                                                            mirrorSessionID: mirrorSession,
                                                            adapterID: config.scenario.adapterID)
                }
                let servedReceipt = try await serveReceipt
                log("receipt", requestReceipt)
                log("receipt_source", servedReceipt)

                mayMutateDest = try destEngine.mayMutate(sessionID: mirrorSession)
                mayMutateSource = try sourceEngine.mayMutate(sessionID: sourceSession)
                log("may_mutate(dest) post-transfer", "\(mayMutateDest)   (owner, new epoch)")
                log("may_mutate(source) post-relinquishment", "\(mayMutateSource)   (read-only replica)")
                emit(.mutationFlipped(side: .destination, session: mirrorSession, may: mayMutateDest))
                emit(.mutationFlipped(side: .source, session: sourceSession, may: mayMutateSource))
                emit(.sessionState(side: .source, session: sourceSession, state: .readOnlyActive))
                if !(mayMutateDest && !mayMutateSource) {
                    // Single-writer flip violated (§21.2) — mirrors DemoRunner.kt:630-636.
                    throw CarryonError(code: CARRYON_E_AUTH_BASE,
                                       message: "single-writer flip violated (§21.2)")
                }
                if let v = parseAnyJSON(requestReceipt), case .object(let o) = v,
                   let epoch = o["new_epoch"]?.numberValue {
                    newEpoch = UInt64(epoch)
                }
            } else {
                // Honest skip: org.carryon.graph declares supports_authority_transfer:false.
                let note = "org.carryon.graph declares supports_authority_transfer:false — L4 single-writer transfer is the editor flagship (§2 labels honest)"
                log("skipped", note)
                mayMutateDest = false
                mayMutateSource = true
                authorityOutcome = .skipped(note)
            }
            emit(.stageFinished(.authority, authorityOutcome))

            // ---- 7. EDIT --------------------------------------------------------
            emit(.stageStarted(.edit))
            logPlain("stage 7/8:  EDIT")
            // Disconnect the transport before the continuation edit (source-off, as
            // carryon_l3.c does): receipts are durable; the edit runs store-local.
            let finalBytes = try destSession.byteCounters()
            let hops = allSessions // snapshot: detached tasks capture lets, not vars
            try await det("session_teardown") {
                for s in hops { s.teardown() } // every hop, including retries
            }
            log("transport", "disconnected (source-off continuation)")
            emit(.counters(sent: finalBytes.sent, recv: finalBytes.recv))
            // Source-off means the SOURCE ENGINE closes too, not just the
            // transport: authority has moved (editor) or the handoff is complete
            // (graph), the receipts are durable, and the destination continues
            // alone. Leaving the engine context alive would make "source-off"
            // a half-truth (review finding: engine outlived the disconnect).
            try await det("core_free_source") { sourceEngine.close() }
            log("source_engine", "closed — destination continues alone")

            let tEditStart = nowMs()
            let edit = try await det("execute_action_as") {
                try destEngine.executeActionAs(
                    cut: CutRef(session: mirrorSession, number: mirrorCut),
                    class: config.scenario.editClass, params: config.scenario.editParams,
                    adapterID: config.scenario.adapterID)
            }
            let editMs = nowMs() - tEditStart
            log("action", config.scenario.editClass)
            log("output_hash", edit.output_hash)
            log("oracle", "agreed=\(edit.oracleAgreed) checked=\(edit.oracle.checked)")
            if !edit.oracleAgreed {
                // Independent-edit oracle disagreed — mirrors DemoRunner.kt:666-675
                // (E_ACTION_BASE+3). Safe for the graph scenario too: its oracle
                // always reports checked:true, and a checked disagreement already
                // fails in the core first (OracleFailure).
                throw CarryonError(code: CARRYON_E_ACTION_BASE + 3,
                                   message: "independent-edit oracle disagreed")
            }
            if !config.scenario.supportsAuthorityTransfer {
                log("note", "may_mutate stays false — \(config.scenario.editClass) is a read-only computation (no authority moved)")
            }
            emit(.stageFinished(.edit, .ok))

            // ---- 8. EVIDENCE ----------------------------------------------------
            emit(.stageStarted(.evidence))
            logPlain("stage 8/8:  EVIDENCE")
            let level = config.scenario == .editorL3 ? "L3-structured-continuation" : "L2-graph-handoff"
            let extra: AnyJSON = .object([
                "level": .string(level),
                "mirror_session_id": .string(mirrorSession),
                "mirror_cut": .number(Double(mirrorCut)),
                "source_session_id": .string(sourceSession),
                "bytes_transferred_sent": .number(Double(finalBytes.sent)),
                "bytes_transferred_recv": .number(Double(finalBytes.recv)),
                "bytes_recv_to_action_ready": .number(Double(bytesAfterImport.recv)),
                "time_to_action_ready_ms": .number(timeToActionReadyMs),
                "restore_latency_ms": .number(restoreLatencyMs),
                "source_independence_ms": .number(editMs),
                "independent_edit_ms": .number(editMs),
                "authoritative_objects": .array(config.scenario == .editorL3
                    ? [.string("editor.document.v1"), .string("editor.unsaved_edits.v1"), .string("editor.meta.v1")]
                    : [.string("graph.sample.v1")]),
                "optional_objects": .array(config.scenario == .editorL3
                    ? [.string("editor.navigation.v1")] : []),
                "oracle_restore_agreed": .bool(restoreAgreed),
                "oracle_independent_edit_agreed": .bool(edit.oracleAgreed),
                "authority_transfer": .string(config.scenario.supportsAuthorityTransfer
                    ? "single-writer; durable receipts before epoch move (§21.2)"
                    : "skipped (adapter declares supports_authority_transfer:false)"),
                "endpoint_changed_symbols_note": .string("separate symbol-distance metric (spec 5.6); not bytes/runtime; see mathview"),
            ])
            let bundleJSON = try await det("export_evidence_with") {
                try destEngine.exportEvidence(sessionID: mirrorSession, extraMetricsJSON: extra.jsonEncoded)
            }
            let evidencePath = dstDir + "/demo-evidence.json"
            try bundleJSON.write(toFile: evidencePath, atomically: true, encoding: .utf8)
            let verify = try await det("verify_evidence") {
                try destEngine.verifyEvidence(atPath: evidencePath)
            }
            log("bundle", evidencePath)
            log("metrics", "level=\(level) sent=\(finalBytes.sent) recv=\(finalBytes.recv) tta_ms=\(String(format: "%.1f", timeToActionReadyMs))")
            log("verify", "{\"ok\":\(verify.ok),\"message\":\"\(verify.message)\"}")
            emit(.stageFinished(.evidence, verify.ok ? .ok : .failed("verify ok:\(verify.ok)")))

            let elapsed = nowMs() - t0
            logBlank()
            logPlain(String(format: "done:  8/8 stages in %.1f ms", elapsed))
            logBlank()

            // Fixed footer, verbatim from plan.md, then the result.
            for l in DemoResult.disclosureLines {
                logPlain(l)
            }
            let result = DemoResult(
                ok: verify.ok,
                scenario: config.scenario,
                workRoot: root,
                evidencePath: evidencePath,
                bundleJSON: bundleJSON,
                bundle: EvidenceBundle.parse(bundleJSON),
                verifyOK: verify.ok,
                verifyMessage: verify.message,
                sourceSessionID: sourceSession,
                mirrorSessionID: mirrorSession,
                mirrorCut: mirrorCut,
                newEpoch: newEpoch,
                sourcePin: sourcePin,
                destPin: destPin,
                bytesSent: finalBytes.sent,
                bytesRecv: finalBytes.recv,
                mayMutateDestination: mayMutateDest,
                mayMutateSource: mayMutateSource,
                localTrustJSON: pairing.localTrustJSON,
                remoteTrustJSON: pairing.remoteTrustJSON,
                sourceTimeline: [.preparing, .cutProposed, .cutSealed, .importing, .readOnlyActive],
                destTimeline: [.preparing, .importing, .actionReady],
                stageOutcomes: [
                    .open: .ok, .identity: .ok, .pair: .ok, .session: .ok,
                    .transfer: .ok, .authority: authorityOutcome, .edit: .ok,
                    .evidence: verify.ok ? .ok : .failed("verify ok:\(verify.ok)"),
                ],
                console: lines,
                elapsedMS: elapsed,
                failure: nil)
            emit(.finished(result))
            return result
        } catch {
            // Failure paths never free sessions (handles leak per the macOS
            // convention). Nothing to join: no detached task touches a session
            // concurrently — counters are sampled between operations only.
            throw error
        }
    }
}

/// Static helpers that need no core handle.
private enum CoreStatic {
    static func mirrorSessionID(remoteSession: String) throws -> String {
        try outString("carryon_mirror_session_id") { out in
            remoteSession.withCString { carryon_mirror_session_id($0, out) }
        }
    }
}
