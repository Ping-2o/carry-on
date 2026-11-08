/*
 * DemoExample/main.swift — minimal "demo mode" over the CarryOnKit SDK: the
 * full 8-stage pipeline (open → identity → pair → session → transfer →
 * authority → edit → evidence) through the real engine C ABI, two cores,
 * loopback TLS 1.3.
 *
 * OBSERVED (LOCAL) evidence — one process, one machine, loopback. Exits 0 iff
 * the evidence bundle verifies {"ok":true,...}.
 */

import CarryOnKit
import Foundation

// The ABI gate: fail with an actionable message, never a random E_PROTO.
try CarryOnSDK.requireCompatibleEngine()
print("carryon-sdk:  \(CarryOnSDK.version)  (engine ABI \(CarryOnSDK.abiMajor).\(CarryOnSDK.abiMinor) verified)")

var config = DemoConfig()
var args = Array(CommandLine.arguments.dropFirst())
if let scenario = args.first {
    switch scenario {
    case "editor": config.scenario = .editorL3
    case "graph": config.scenario = .graph
    default: break
    }
}

let rc = try awaitSync { await DemoConsole.runHeadless(config: config) }
print("demo example:  \(rc == 0 ? "OK — evidence verified" : "FAILED")  (LOCAL, spec §2/§30)")
exit(Int32(rc))

private final class SyncBox<T>: @unchecked Sendable {
    var value: T?
    var error: Error?
}

func awaitSync<T>(_ body: @escaping () async throws -> T) throws -> T {
    let box = SyncBox<T>()
    let sem = DispatchSemaphore(value: 0)
    Task.detached {
        do { box.value = try await body() } catch { box.error = error }
        sem.signal()
    }
    sem.wait()
    if let e = box.error { throw e }
    return box.value!
}
