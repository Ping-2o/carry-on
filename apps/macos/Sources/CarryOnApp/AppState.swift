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

/// One PREPARED session in the "Transfer session" flow (stage 6): a sealed
/// cut («активная сессия», §16.2 CutSealed) waiting to be transferred, or its
/// post-transfer record once served.
@MainActor
@Observable
final class ArmedSession: Identifiable {
    let id = UUID()
    let prepared: PreparedHandoff
    let preparedAt = Date()
    /// Exact §16.2 state of the SOURCE side: .cutSealed while armed;
    /// .importing while a transfer leg runs; .actionReady after ImportComplete;
    /// .failed when the transfer failed (interrupted ≠ success, §19.3.9).
    var state: SessionState = .cutSealed
    var servedResult: FileHandoffResult?

    var title: String { prepared.fileName }
    var subtitle: String { "cut=\(prepared.cutNumber) · \(prepared.fileBytes) B" }

    init(prepared: PreparedHandoff) {
        self.prepared = prepared
    }
}

/// The "Transfer session" model (stage 6): PREPARE an armed session from a
/// chosen file (§16.2 Preparing⇒CutSealed), then TRANSFER it to a discovered
/// receiver with the PAIRED Keychain trust (stage 5) — the manual typed pin
/// stays the fallback. Live states + console lines are applied from pipeline
/// events on the main actor; counters are sampled only between completed
/// wire ops; the serve runs with resume legs (cooperative §11.2 suspend on
/// the destination re-serves on a fresh connection).
@MainActor
@Observable
final class CarryModel {
    var running = false
    var preparing = false
    var consoleLines: [ConsoleLine] = []
    var ladderStates: [HandoffLadderStep: StageOutcome] = [:]
    var sessionState: SessionState?
    var bytesSent: UInt64 = 0
    var bytesRecv: UInt64 = 0
    var lastResult: FileHandoffResult?
    var lastError: String?
    var evidenceClass: String?
    /// The armed sessions ("активные сессии") — CutSealed, transferable.
    var armedSessions: [ArmedSession] = []
    /// The armed session selected for transfer.
    var selectedArmedID: UUID?
    /// The durable identity's full pin (public fingerprint; clipboard only,
    /// never logged — EVD-006 excludes only private keys).
    var sourcePinHex = ""

    // ---- hand-made configuration (persisted caching, stage-3/4 kept) ----
    var fileURL: URL? {
        didSet { UserDefaults.standard.set(fileURL?.path, forKey: "carryon.handoff.lastFile") }
    }
    /// Receiver address: filled from the engine's discovered list or typed by
    /// hand (manual fallback, cached). Hand edits clear the discovery claim.
    var receiverAddr: String {
        didSet {
            UserDefaults.standard.set(receiverAddr, forKey: "carryon.handoff.lastAddr")
            discoveryLatencyMS = nil
        }
    }
    /// The receiver's 64-hex pin — the MANUAL FALLBACK only. Ignored when
    /// `usePairedTrust` (the stage-5 Keychain store owns the trust then).
    var receiverPin = ""
    /// Transfer with the PERSISTED paired trust (stage 5) instead of the
    /// t

[Truncated after 209 lines. To continue, call readFile again with offset=210.]