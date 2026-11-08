/*
 * main.swift — headless source side of the L0/L1 file handoff (stage 3 →
 * stage 6): the REAL engine through the C ABI, driving the SAME CarryOnKit
 * code the macOS app's "Transfer session" screen uses.
 *
 *   carryon-filehandoff-smoke serve <file> <receiver-addr> <receiver-pin> [--tamper]
 *   carryon-filehandoff-smoke serve <file> --discovered [--expect-dc mobile]
 *        [--paired] [--legs N] [--tamper] [--json]
 *
 * The stage-6 chain (--discovered --paired --legs 3): the receiver address
 * comes from the ENGINE's browse (discovery, §18.3 — never typed), the trust
 * is the PERSISTED Keychain store (stage-5 pairing — no pin typed), and the
 * serve runs with resume legs so a cooperative §11.2 suspend on the
 * destination is survivable. `<receiver-addr>`/`<receiver-pin>` positional
 * mode stays the honest manual fallback.
 *
 * Prints its full pairing pin (a public fingerprint) BEFORE connecting, then
 * serves the sealed cut over LAN TLS 1.3 until the receiver reports
 * ImportComplete (retrying resume legs as the planner decides). Exits 0 only
 * if the source evidence bundle verifies {"ok":true,...}.
 *
 * Labels (§2 law): loopback receiver → OBSERVED (LOCAL); a LAN receiver on a
 * real second device → PHYSICAL cross-device evidence. Neither is a
 * supported platform (§30 unrun).
 */

import CarryOnKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data(("FAIL: \(message)\n").utf8))
    exit(2)
}

var args = Array(CommandLine.arguments.dropFirst())
guard let role = args.first else {
    fail("usage: carryon-filehandoff-smoke serve <file> <receiver-addr> <receiver-pin> [--tamper] | serve <file> --discovered [--expect-dc mobile] [--paired] [--legs N] [--json]")
}
args.removeFirst()
guard role == "serve" else { fail("unknown role \(role) — expected 'serve'") }
guard let file = args.first, !file.hasPrefix("--") else { fail("serve needs <file> first") }
args.removeFirst()

// Positional manual mode: <addr> <pin>.
var manualAddr: String?
var manualPin: String?
if let a = args.first, !a.hasPrefix("--") {
    manualAddr = a
    args.removeFirst()
    if let p = args.first, !p.hasPrefix("--") {
        manualPin = p
        args.removeFirst()
    }
}

var discovered = false
var expectDC: String? = nil
var paired = false
var legs = 1
var jsonOut = false
while let flag = args.first {
    args.removeFirst()
    switch flag {
    case "--tamper": break // handled through the config below
    case "--discovered": discovered = true
    case "--expect-dc":
        guard let v = args.first else { fail("--expect-dc needs a value") }
        args.removeFirst()
        expectDC = v
    case "--paired": paired = true
    case "--legs":
        guard let v = args.first, let n = Int(v), n > 0 else { fail("--legs needs a positive int") }
        args.removeFirst()
        legs = n
    case "--json": jsonOut = true
    default: fail("unknown flag \(flag)")
    }
}
let tamper = role == "serve" && CommandLine.arguments.contains("--tamper")

let fileURL = URL(fileURLWithPath: file)
guard FileManager.default.fileExists(atPath: file) else { fail("no such file: \(file)") }

// ---- stage-6 chain: the ENGINE discovers the receiver (§18.3) -----------
var receiverAddr = manualAddr ?? ""
var discoveryMS: Double?
if discovered {
    // Synchronous engine browse (the same EngineDiscoveryBrowser the UI model
    // wraps) — no async top-level, so `dispatchMain()` below stays legal.
    guard let browser = try? EngineDiscoveryBrowser() else {
        fail("discovery: the engine browser could not start")
    }
    print("discovery:  browsing _carryon._tcp for a receiver…")
    fflush(stdout)
    let t0 = DispatchTime.now().uptimeNanoseconds
    let deadline = Date().addingTimeInterval(10)
    var found: EnginePeer?
    while found == nil && Date() < deadline {
        let snap = (try? browser.snapshot())?.peers ?? []
        found = expectDC == nil ? snap.first : snap.first { $0.txt.device_class == expectDC }
        if found == nil { Thread.sleep(forTimeInterval: 0.3) }
    }
    browser.stop()
    guard let peer = found else {
        fail("discovery found no receiver within 10 s (expect-dc=\(expectDC ?? "any"))")
    }
    let ms = Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6
    receiverAddr = peer.address
    discoveryMS = ms
    print(String(format: "discovery:  %@  dc=%@ pm=%d → %@  (first peer %.0f ms, NET-003 %@)",
                 peer.instanceName, peer.txt.device_class, peer.txt.protocol_major,
                 peer.address, ms, ms < 10_000 ? "PASS" : "FAIL"))
    fflush(stdout)
}

// ---- stage-5 chain: the PERSISTED paired trust (Keychain) ---------------
var trustJSON: String?
var durableIdentity: DeviceIdentity?
if paired {
    guard let store = SecureStore.loadTrust() else {
        fail("--paired: no persisted trust in the Keychain — pair first (carryon-pairing-smoke join / the Pairing pane)")
    }
    guard let json = try? store.toJSON() else { fail("--paired: trust store serialization failed") }
    trustJSON = json
    durableIdentity = try? SecureStore.loadOrCreateIdentity(name: "carryon-mac-smoke")
    print(String(format: "trust:      PERSISTED paired store (%d record(s)) — unpaired/revoked receivers fail the handshake closed (§18.5)",
                 (try? store.records().count) ?? 0))
}

// The identity: durable (paired path) or per-run (manual mode).
let identity: DeviceIdentity
if let durable = durableIdentity {
    identity = durable
    print("source_pin:  \(try identity.pinHex())  (durable — Keychain, PLAT-002)")
} else {
    identity = try DeviceIdentity.generate(name: "carryon-mac-smoke")
    print("source_pin:  \(try identity.pinHex())  (full 64-hex; type/paste on the receiver)")
}
fflush(stdout)

// ---- manual-mode validation ----------------------------------------------
if trustJSON == nil {
    guard let addr = manualAddr, HandoffValidation.isValidReceiverAddr(addr) else {
        fail("receiver address must be IPv4:port (or use --discovered)")
    }
    guard let pin = manualPin, HandoffValidation.isValidPinHex(pin) else {
        fail("manual fallback needs the receiver's 64-hex pin (or pair once and use --paired)")
    }
}

let config = FileHandoffConfig(
    fileURL: fileURL, receiverAddr: receiverAddr, receiverPin: manualPin ?? "",
    identity: identity, integrityNegative: tamper,
    discoveryFirstPeerMS: discoveryMS, trustJSON: trustJSON, serveLegs: legs)

// Sendable copies for the detached closure (Swift-6 discipline ahead).
let jsonOutFlag = jsonOut
let discMSCopy = discoveryMS

Task.detached(priority: .userInitiated) {
    do {
        let result = try await FileHandoffPipeline().run(config) { event in
            switch event {
            case .console(let line):
                print(line.rendered)
                fflush(stdout)
            case .sourcePin:
                break // already printed before the pipeline started
            case .ladder(let step, let outcome):
                print("ladder:  \(step.label) → \(outcome)")
                fflush(stdout)
            case .state(let s):
                print("state:  \(s.rawValue)")
            case .counters(let sent, let recv):
                print("counters:  sent=\(sent) recv=\(recv)")
            case .finished, .failed:
                break
            }
        }
        if jsonOutFlag {
            print(String(format: "json:      {\"ok\":%@,\"session\":\"%@\",\"cut\":%d,\"legs\":%d,\"paired\":%@,\"ms\":%.0f,\"sent\":%d,\"recv\":%d,\"discovery_ms\":%@,\"evidence_class\":\"%@\"}",
                         result.ok ? "true" : "false",
                         result.sessionID, result.cutNumber, result.serveLegsUsed,
                         result.pairedTrust ? "true" : "false", result.elapsedMS,
                         result.bytesSent, result.bytesRecv,
                         discMSCopy.map { String(format: "%.0f", $0) } ?? "null",
                         result.evidenceClass))
        }
        print("done:  transferred \(result.sessionID) cut=\(result.cutNumber) hash=\(result.declaredHash.prefix(16))… legs=\(result.serveLegsUsed) trust=\(result.pairedTrust ? "paired" : "typed") evid=\(result.evidenceClass)")
        exit(Int32(result.verifyOK && result.ok ? 0 : 1))
    } catch {
        FileHandle.standardError.write(Data(("FAIL: \(error)\n").utf8))
        exit(2)
    }
}
dispatchMain() // the detached task exits the process when the handoff settles
