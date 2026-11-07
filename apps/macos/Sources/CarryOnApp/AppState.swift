/*
 * AppState.swift — observable app state: the demo run model (applies pipeline
 * events on the main actor), topology/trust model (persisted trust JSON via
 * carryon_trust_to_json/from_json), and navigation.
 */

import CarryOnKit
import Foundation
import Observation

/// One run's per-stage UI state.
enum StageUIState: Equatable {
    case pending
    case running
    case finished(StageOutcome)

    var outcome: StageOutcome? {
        if case .finished(let o) = self { return o }
        return nil
    }
}

@MainActor
@Observable
final class DemoModel {
    var running = false
    var runningStage: DemoStage?
    var stageStates: [DemoStage: StageUIState] =
        Dictionary(uniqueKeysWithValues: DemoStage.allCases.map { ($0, .pending) })
    var consoleLines: [ConsoleLine] = []
    var bytesSent: UInt64 = 0
    var bytesRecv: UInt64 = 0
    var lastResult: DemoResult?
    var lastError: String?
    var config = DemoConfig()

    // §16.2 session states, per side (never the bare word "Ready").
    private(set) var sourceSessions: [(id: String, state: SessionState)] = []
    private(set) var destSessions: [(id: String, state: SessionState)] = []
    private(set) var mayMutateDest: Bool?
    private(set) var mayMutateSource: Bool?

    /// Run identity: events and completion effects from a superseded pipeline
    /// are dropped by token, so a stale run can never write the shared model
    /// (single-run invariant, §2 label law — mirrors the Android DemoRunner's
    /// whole-pipeline mutex).
    private var runToken = UUID()

    func reset() {
        // Never wipe state under a live run: `running` is the flag run()'s
        // re-entrancy guard reads — clearing it here (⌘N mid-run) would admit
        // a second concurrent pipeline. run() itself resets before starting.
        guard !running else { return }
        running = false
        runningStage = nil
        stageStates = Dictionary(uniqueKeysWithValues: DemoStage.allCases.map { ($0, .pending) })
        consoleLines = []
        bytesSent = 0
        bytesRecv = 0
        lastResult = nil // per-run state: a failed/absent run must not inherit a prior run's outcome (§2/EVD-003)
        lastError = nil
        sourceSessions = []
        destSessions = []
        mayMutateDest = nil
        mayMutateSource = nil
    }

    /// Run the pipeline. Blocking FFI is inside the pipeline (Task.detached);
    /// events are marshaled back to the main actor.
    func run() async {
        guard !running else { return }
        reset()
        running = true
        let token = UUID()
        runToken = token
        defer { if runToken == token { running = false } }
        do {
            let result = try await DemoPipeline().run(config) { [weak self] event in
                let s = self
                let t = token
                Task { @MainActor in
                    s?.apply(event, token: t)
                }
            }
            guard runToken == token else { return }
            lastResult = result
            mayMutateDest = result.mayMutateDestination
            mayMutateSource = result.mayMutateSource
            if result.verifyOK {
                stageStates[.evidence] = .finished(.ok)
            }
        } catch is CancellationError {
            guard runToken == token else { return }
            lastError = "run cancelled"
        } catch {
            guard runToken == token else { return }
            lastError = error.localizedDescription
            if let runningStage {
                stageStates[runningStage] = .finished(.failed(error.localizedDescription))
            }
        }
    }

    private func apply(_ event: DemoEvent, token: UUID) {
        guard token == runToken else { return } // drop superseded runs' events
        switch event {
        case .console(let line):
            consoleLines.append(line)
        case .stageStarted(let stage):
            runningStage = stage
            stageStates[stage] = .running
        case .stageFinished(let stage, let outcome):
            runningStage = nil
            stageStates[stage] = .finished(outcome)
        case .counters(let sent, let recv):
            bytesSent = sent
            bytesRecv = recv
        case .mutationFlipped(let side, _, let may):
            if side == .destination { mayMutateDest = may } else { mayMutateSource = may }
        case .sessionState(let side, let session, let state):
            if side == .source {
                upsert(&sourceSessions, session, state)
            } else {
                upsert(&destSessions, session, state)
            }
        case .finished(let result):
            lastResult = result
        case .failed(let why):
            lastError = why
        }
    }

    private func upsert(_ list: inout [(id: String, state: SessionState)],
                        _ id: String, _ state: SessionState) {
        if let idx = list.firstIndex(where: { $0.id == id }) {
            list[idx].state = state
        } else {
            list.append((id, state))
        }
    }
}

/// The L0/L1 "Carry File" model (manual-IP mode, pre-discovery): the chosen
/// file, the hand-entered receiver address (cached), a pre-armed source
/// identity (so the receiver can pin this Mac BEFORE the serve starts), and
/// the live handoff state applied from pipeline events on the main actor.
@MainActor
@Observable
final class CarryModel {
    var running = false
    var consoleLines: [ConsoleLine] = []
    var ladderStates: [HandoffLadderStep: StageOutcome] = [:]
    var sessionState: SessionState?
    var bytesSent: UInt64 = 0
    var bytesRecv: UInt64 = 0
    var lastResult: FileHandoffResult?
    var lastError: String?
    var evidenceClass: String?

    // ---- hand-made configuration (persisted where the task asks for caching)
    var fileURL: URL? {
        didSet { UserDefaults.standard.set(fileURL?.path, forKey: "carryon.handoff.lastFile") }
    }
    /// Last receiver address typed by hand (cached; discovery is stage 4).
    var receiverAddr: String {
        didSet { UserDefaults.standard.set(receiverAddr, forKey: "carryon.handoff.lastAddr") }
    }
    /// The receiver's 64-hex pin (typed/pasted from its Receive screen).
    var receiverPin = ""
    /// Integrity negative test toggle (declares a tampered hash; the receiver
    /// MUST refuse — the visible-failure counterpart of the §6.8 showcase).
    var integrityNegative = false
    /// Pre-armed identity: created once so its pin exists before serving.
    private(set) var sourceIdentity: DeviceIdentity?
    private(set) var sourcePinHex = ""
    private(set) var sourcePinTruncated = ""

    private var runToken = UUID()

    init() {
        let d = UserDefaults.standard
        if let p = d.string(forKey: "carryon.handoff.lastFile"), !p.isEmpty,
           FileManager.default.fileExists(atPath: p) {
            fileURL = URL(fileURLWithPath: p)
        }
        receiverAddr = d.string(forKey: "carryon.handoff.lastAddr") ?? ""
        Task { await makeIdentity() }
    }

    /// A fresh identity per arm (honest: pins are per-handoff fingerprints;
    /// long-lived device identities + revocation are stage-5 pairing).
    func rearmIdentity() {
        Task { await makeIdentity() }
    }

    private func makeIdentity() async {
        do {
            let id = try DeviceIdentity.generate(name: "carryon-mac-carry")
            sourceIdentity = id
            sourcePinHex = try id.pinHex()
            sourcePinTruncated = String(sourcePinHex.prefix(12)) + "…"
        } catch {
            sourcePinHex = ""
            sourcePinTruncated = ""
        }
    }

    var configValid: Bool {
        guard let file = fileURL,
              HandoffValidation.isValidReceiverAddr(receiverAddr),
              HandoffValidation.isValidPinHex(receiverPin) else { return false }
        return FileManager.default.fileExists(atPath: file.path)
    }

    var addrValid: Bool { HandoffValidation.isValidReceiverAddr(receiverAddr) }
    var pinValid: Bool { HandoffValidation.isValidPinHex(receiverPin) }

    func reset() {
        guard !running else { return }
        consoleLines = []
        ladderStates = [:]
        sessionState = nil
        bytesSent = 0
        bytesRecv = 0
        lastResult = nil // per-run state: a failed handoff must not inherit a prior result (§2/EVD-003)
        lastError = nil
        evidenceClass = nil
    }

    /// Serve the chosen file to the manually-entered receiver. Blocking FFI
    /// is inside the pipeline (Task.detached); events are marshaled here.
    func run() async {
        guard !running, configValid, let file = fileURL, let identity = sourceIdentity else { return }
        reset()
        running = true
        let token = UUID()
        runToken = token
        defer { if runToken == token { running = false } }
        let config = FileHandoffConfig(
            fileURL: file, receiverAddr: receiverAddr, receiverPin: receiverPin,
            identity: identity, integrityNegative: integrityNegative)
        do {
            let result = try await FileHandoffPipeline().run(config) { [weak self] event in
                let s = self
                let t = token
                Task { @MainActor in
                    s?.apply(event, token: t)
                }
            }
            guard runToken == token else { return }
            lastResult = result
            evidenceClass = result.evidenceClass
        } catch is CancellationError {
            guard runToken == token else { return }
            lastError = "handoff cancelled"
        } catch {
            guard runToken == token else { return }
            lastError = error.localizedDescription
            // Mark the first not-yet-settled ladder step failed (honest UI).
            if let step = HandoffLadderStep.allCases.first(where: { ladderStates[$0] == nil }) {
                ladderStates[step] = .failed(error.localizedDescription)
            } else {
                ladderStates[.l0Carry] = .failed(error.localizedDescription)
            }
        }
    }

    private func apply(_ event: HandoffEvent, token: UUID) {
        guard token == runToken else { return } // drop superseded runs' events
        switch event {
        case .console(let line):
            consoleLines.append(line)
        case .sourcePin:
            break // the pre-armed identity's pin is already in sourcePinHex
        case .ladder(let step, let outcome):
            ladderStates[step] = outcome
        case .state(let s):
            sessionState = s
        case .counters(let sent, let recv):
            bytesSent = sent
            bytesRecv = recv
        case .finished(let result):
            lastResult = result
            evidenceClass = result.evidenceClass
        case .failed(let why):
            lastError = why
        }
    }
}

@MainActor
@Observable
final class TopologyModel {
    /// Ephemeral local identity for the pilot card. The KEY DER is SECRET
    /// (§22.3): it lives only in this process — never persisted, never logged.
    private(set) var localIdentity: DeviceIdentity?
    private(set) var localPin: String = ""

    // Trust store persisted as JSON (pins are fingerprints, not secrets).
    var trust: TrustStore?
    private(set) var records: [TrustRecord] = []
    private var trustJSON: String {
        didSet { UserDefaults.standard.set(trustJSON, forKey: "carryon.trust.json") }
    }

    let discovery = DiscoveryPilot()

    init() {
        trustJSON = UserDefaults.standard.string(forKey: "carryon.trust.json") ?? "{\"peers\":{}}"
        reloadTrust()
        Task { await makeIdentity() }
    }

    private func makeIdentity() async {
        do {
            let id = try DeviceIdentity.generate(name: Host.current().localizedName ?? "mac")
            localIdentity = id
            localPin = try id.pinHex()
        } catch {
            localPin = ""
        }
    }

    private func reloadTrust() {
        if let t = try? TrustStore.fromJSON(trustJSON) {
            trust = t
            records = (try? t.records()) ?? []
        }
    }

    /// Merge the paired trust stores produced by a demo run into the persisted one.
    /// Merge INTO the persisted store: prior pins — including revoked records
    /// (revoked=true + bumped trust_generation, the durable §18.4.9 carrier) —
    /// survive every demo run. (Android sibling: TopologyStore.recordDemoPairing.)
    func absorbPairing(localJSON: String, remoteJSON: String) {
        // Seed from the CURRENT persisted store, never from the fresh pairing
        // halves — each half is a brand-new single-pin store (pairing.rs
        // pair_devices), so seeding from it would replace the whole ledger.
        let baseJSON = trustJSON.isEmpty ? "{\"peers\":{}}" : trustJSON
        guard let merged = try? TrustStore.fromJSON(baseJSON) else { return }
        // Snapshot the persisted records so revoked pins can be detected below.
        let persisted = (try? merged.records()) ?? []
        // The peer record from the remote half is what THIS device keeps about
        // the peer; pair both halves' records into the persisted store by pin.
        for half in [localJSON, remoteJSON] {
            guard let store = try? TrustStore.fromJSON(half) else { continue }
            for r in (try? store.records()) ?? [] where !r.pin_hex.isEmpty {
                // carryon_trust_pair OVERWRITES an existing record with
                // revoked=false/trust_generation=0 — never resurrect a
                // revoked pin (§18.4.9 fail-closed).
                if let existing = persisted.first(where: { $0.pin_hex == r.pin_hex }),
                   existing.revoked { continue }
                _ = try? merged.pair(name: r.device_name, pinHex: r.pin_hex, utc: r.paired_utc)
            }
        }
        trust = merged
        if let json = try? merged.toJSON() {
            trustJSON = json
        }
        records = (try? merged.records()) ?? []
    }

    /// Revoke by pin (§18.4.9) and persist (EVD-006-clean: pins only).
    func revoke(_ record: TrustRecord) {
        guard let trust else { return }
        _ = try? trust.revoke(pinHex: record.pin_hex)
        if let json = try? trust.toJSON() {
            trustJSON = json
        }
        records = (try? trust.records()) ?? []
    }
}

@MainActor
@Observable
final class AppState {
    var pane: Pane = .demo
    var demo = DemoModel()
    var topology = TopologyModel()
    var carry = CarryModel()
    /// The last evidence bundle text (from a run or a loaded file) for the viewer.
    var evidenceJSON: String?

    /// Run the demo, then feed the pairing into the topology (trust records
    /// hold pins only — no secrets, EVD-006-clean).
    func runDemo() async {
        await demo.run()
        // Fail closed: only a run that completed without error may absorb its
        // pairing and publish its evidence bundle. lastError is set on every
        // failure path (thrown error or cancellation); a failed run must not
        // re-present the previous run's result (§2/EVD-003).
        guard demo.lastError == nil, let r = demo.lastResult else { return }
        topology.absorbPairing(localJSON: r.localTrustJSON,
                               remoteJSON: r.remoteTrustJSON)
        evidenceJSON = r.bundleJSON
    }

    /// ⌘N — prepare a fresh run state and focus the Sessions pane.
    /// Model-level refusal while a run is in flight (control-equivalence with
    /// the Android sibling's DemoRunner: no state mutation mid-run).
    func newSession() {
        guard !demo.running else { return }
        demo.reset()
        pane = .sessions
    }

    enum Pane: String, CaseIterable, Identifiable, Hashable {
        case demo = "Demo"
        case carry = "Carry File"
        case topology = "Topology"
        case sessions = "Sessions"
        case evidence = "Evidence"
        var id: String { rawValue }

        var symbol: String {
            switch self {
            case .demo: return "play.rectangle"
            case .carry: return "paperplane"
            case .topology: return "point.3.connected.trianglepath.dotted"
            case .sessions: return "rectangle.stack"
            case .evidence: return "checkmark.seal"
            }
        }
    }
}
