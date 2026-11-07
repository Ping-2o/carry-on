/*
 * main.swift — headless source side of the L0/L1 file handoff (stage 3,
 * manual-IP mode): the REAL engine through the C ABI, driving the SAME
 * FileHandoffPipeline the macOS app's "Carry File" screen uses.
 *
 *   carryon-filehandoff-smoke serve <file> <receiver-addr> <receiver-pin> [--tamper]
 *
 * Prints its full pairing pin (a public fingerprint) BEFORE connecting, so the
 * operator can arm the receiver, then serves the sealed file cut over LAN
 * TLS 1.3 until the receiver reports ImportComplete. Exits 0 only if the
 * source evidence bundle verifies {"ok":true,...}.
 *
 * Labels (§2 law): loopback receiver → OBSERVED (LOCAL); a LAN receiver on a
 * real second device → PHYSICAL cross-device evidence (e.g. the 2026-11-07
 * Galaxy A04 run). Neither is a supported platform (§30 unrun).
 */

import CarryOnKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data(("FAIL: \(message)\n").utf8))
    exit(2)
}

var args = Array(CommandLine.arguments.dropFirst())
guard let role = args.first else {
    fail("usage: carryon-filehandoff-smoke serve <file> <receiver-addr> <receiver-pin> [--tamper]")
}
args.removeFirst()
guard role == "serve" else { fail("unknown role \(role) — expected 'serve'") }
guard args.count >= 3 else { fail("serve needs <file> <receiver-addr> <receiver-pin>") }
let file = args.removeFirst()
let addr = args.removeFirst()
let pin = args.removeFirst()
let tamper = args.contains("--tamper")

let fileURL = URL(fileURLWithPath: file)
guard FileManager.default.fileExists(atPath: file) else { fail("no such file: \(file)") }

// Pre-generate the identity so the full pin is printed BEFORE the blocking
// connect — the receiver must be able to pin this Mac first (manual mode).
let identity: DeviceIdentity
do {
    identity = try DeviceIdentity.generate(name: "carryon-mac-smoke")
    print("source_pin:  \(try identity.pinHex())  (full 64-hex; type/paste on the receiver)")
} catch {
    fail("identity: \(error)")
}

let config = FileHandoffConfig(
    fileURL: fileURL, receiverAddr: addr, receiverPin: pin,
    identity: identity, integrityNegative: tamper)

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
        print("done:  served \(result.sessionID) cut=\(result.cutNumber) hash=\(result.declaredHash.prefix(16))… evid=\(result.evidenceClass)")
        exit(Int32(result.verifyOK && result.ok ? 0 : 1))
    } catch {
        FileHandle.standardError.write(Data(("FAIL: \(error)\n").utf8))
        exit(2)
    }
}
dispatchMain() // the detached task exits the process when the handoff settles
