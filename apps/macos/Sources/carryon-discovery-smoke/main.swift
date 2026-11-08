/*
 * main.swift — headless engine-side discovery gate (stage 4, NET-003): the
 * REAL engine through the C ABI — advertise presence (engine TXT contract),
 * browse `_carryon._tcp`, and measure the wall-clock time from browse start to
 * the FIRST ENGINE-VALIDATED peer. The TXT is built and validated by the
 * engine (§18.3: foreign keys ignored, malformed records dropped, pm filtered);
 * this tool only renders what the engine reports.
 *
 *   carryon-discovery-smoke [--expect-dc mobile|desktop] [--timeout 10]
 *                          [--no-advertise] [--json]
 *
 * Exits 0 iff a peer was found (and matched --expect-dc when given) within
 * --timeout seconds; prints every peer, the engine's fail-closed rejections,
 * and the NET-003 timing:
 *
 *   net003:  first peer in 1234 ms (gate < 10000 ms) — PASS
 *
 * `--hold N` keeps advertising+browsing for N seconds (presence window for
 * the OTHER device's NET-003 measurement — the Android app's NSD browse finds
 * this Mac while the hold is up), reporting the last snapshot on the way out.
 *
 * Labels (§2 law): a self-advertised instance found on one machine is
 * OBSERVED (LOCAL); finding the Galaxy A04 (a real second device, dc=mobile)
 * over the LAN is PHYSICAL discovery evidence. Neither is a supported
 * platform (§30 unrun). Recorded in docs/evidence/ with the physical runs.
 */

import CarryOnKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data(("FAIL: \(message)\n").utf8))
    exit(2)
}

var args = Array(CommandLine.arguments.dropFirst())
var expectDC: String?
var timeout: Double = 10
var advertise = true
var json = false
var hold: Double = 0
while !args.isEmpty {
    let flag = args.removeFirst()
    switch flag {
    case "--expect-dc":
        guard let v = args.first else { fail("--expect-dc needs a value") }
        args.removeFirst()
        expectDC = v
    case "--timeout":
        guard let v = args.first else { fail("--timeout needs a value") }
        args.removeFirst()
        timeout = Double(v) ?? 10
    case "--no-advertise": advertise = false
    case "--json": json = true
    case "--hold":
        guard let v = args.first else { fail("--hold needs a value") }
        args.removeFirst()
        hold = Double(v) ?? 0
    case "--hold":
        guard let v = args.first else { fail("--hold needs a value") }
        args.removeFirst()
        hold = Double(v) ?? 0
    default: fail("unknown flag \(flag)")
    }
}

var wantDC = expectDC
var jsonOut = json
var holdSeconds = hold

Task { @MainActor in
    let discovery = EngineDiscovery()

    print("engine discovery — _carryon._tcp (§18.3; TXT built + validated by the engine)")
    if advertise {
        // Presence advertise: the Android app's NSD browse finds this Mac.
        guard discovery.startAdvertising(pairingRequired: true, port: 0) else {
            fail("advertise: \(discovery.lastError)")
        }
        print("advertise:  \(discovery.advertisedName) (desktop, presence; ch=none)")
    }
    discovery.startBrowsing()
    print("browse:     started (monotonic clock; NET-003 gate < 10 s)")
    fflush(stdout)

    let deadline = Date().addingTimeInterval(holdSeconds > 0 ? max(holdSeconds, timeout) : timeout)
    var sawFirst = false

    Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { timer in
        MainActor.assumeIsolated {
            if let ms = discovery.firstPeerLatencyMs, !sawFirst {
                sawFirst = true
                print(String(format: "net003:    first peer in %.0f ms (gate < 10000 ms) — %@",
                             ms, ms < 10_000 ? "PASS" : "FAIL"))
                fflush(stdout)
            }
            if discovery.lastError.contains("snapshot") {
                fail("snapshot: \(discovery.lastError)")
            }
            let matched = wantDC == nil
                ? !discovery.peers.isEmpty
                : discovery.peers.contains { $0.txt.device_class == wantDC }
            // --hold: stay up for the full window (a presence advertiser for
            // the OTHER device's NET-003 measurement), then report and exit.
            if (matched && holdSeconds == 0) || Date() > deadline {
                timer.invalidate()
                print("peers:     \(discovery.peers.isEmpty ? "none" : "")")
                for p in discovery.peers {
                    print("◦  \(p.instanceName)  dc=\(p.txt.device_class) pm=\(p.txt.protocol_major) pr=\(p.txt.pairing_required) ch=\(p.txt.connection_hint)  →  \(p.address)")
                }
                if !discovery.rejections.isEmpty {
                    print("filtered:  (fail-closed, never shown as peers)")
                    for r in discovery.rejections {
                        print("✗  \(r)")
                    }
                }
                if let ms = discovery.firstPeerLatencyMs {
                    if jsonOut {
                        let peers = discovery.peers.map { p in
                            "{\"instance\":\"\(p.instanceName)\",\"address\":\"\(p.address)\",\"dc\":\"\(p.txt.device_class)\",\"pm\":\(p.txt.protocol_major)}"
                        }.joined(separator: ",")
                        print("json:      {\"net003_ms\":\(Int(ms)),\"gate_ms\":10000,\"passed\":\(ms < 10_000),\"peers\":[\(peers)]}")
                    }
                    print(String(format: "net003:    first peer in %.0f ms — %@ (monotonic clock, §18.10)",
                                 ms, ms < 10_000 ? "PASS" : "FAIL"))
                }
                discovery.stopAll()
                let ok = matched && (discovery.firstPeerLatencyMs ?? 1e9) < 10_000
                exit(ok ? 0 : 1)
            }
        }
    }
}
RunLoop.main.run() // services the MainActor task AND the poll timer
