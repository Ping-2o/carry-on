/*
 * CarryFileExample/main.swift — minimal "carry a file" over the CarryOnKit
 * SDK: a real file captured by the compiled-in file adapter (§10.7
 * content-hash identity), sealed cut, loopback TLS 1.3 to an in-process
 * destination, descriptor over the Hello/Welcome features, import, the
 * CORE-004 integrity gate (measured sha256 == declared), and the engine's
 * activation-only document.open record.
 *
 * Loopback = OBSERVED (LOCAL) evidence (one process, one machine). The same
 * code carried across a real LAN to a real device is OBSERVED (PHYSICAL) —
 * see apps/android/evidence/20261108-a04-l0l1/ for that run's evidence.
 */

import CarryOnKit
import Foundation

// The ABI gate: fail with an actionable message, never a random E_PROTO.
try CarryOnSDK.requireCompatibleEngine()
print("carryon-sdk:  \(CarryOnSDK.version)  (engine ABI \(CarryOnSDK.abiMajor).\(CarryOnSDK.abiMinor) verified)")

let root = NSTemporaryDirectory() + "carryon-carryfile-example-\(Int(Date().timeIntervalSince1970))"
try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(atPath: root) }

// The real carried file.
let fileURL = URL(fileURLWithPath: root + "/carryon-doc.png")
try Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] + (0..<128).map { $0 })
    .write(to: fileURL)
let fileHash = HandoffValidation.sha256Hex(of: try Data(contentsOf: fileURL))
print("file:  \(fileURL.lastPathComponent)  sha256=\(fileHash.prefix(16))…")

// ---- destination: core, file adapter (placeholder path), identity + trust,
// listener on loopback, and the receive logic (the same steps the Android
// Receive screen performs; the example mirrors it step for step).
let dstDir = root + "/dest"
let placeholder = "receiver store placeholder"
try placeholder.write(toFile: root + "/placeholder.txt", atomically: true, encoding: .utf8)
let dstEngine = try Engine.open(dataDir: dstDir)
_ = try dstEngine.registerAdapter(id: "org.carryon.file",
                                  paramsJSON: "{\"path\":\"\(root + "/placeholder.txt")\"}")
let dstIdentity = try DeviceIdentity.generate(name: "example-dest")
let dstPin = try dstIdentity.pinHex()
// The source identity is pre-generated so its pin exists BEFORE the accept
// (mutual pinning, §18.5) — exactly what the app's pre-armed identity is for.
let srcIdentity = try DeviceIdentity.generate(name: "example-src")
let srcPin = try srcIdentity.pinHex()
let dstTrust = try TrustStore.fromJSON("{\"peers\":{}}")
try dstTrust.pair(name: "source", pinHex: srcPin,
                  utc: ISO8601DateFormatter().string(from: Date()))
let listener = try NetListener.bind("127.0.0.1:0")
let dstAddr = try listener.addr()

final class Destination: @unchecked Sendable {
    var verified = false
    var opened = false
    var error: String?
}
let destination = Destination()
let acceptTask = Task.detached {
    do {
        let wire = try NetSession.accept(listener: listener, identity: dstIdentity, trust: dstTrust)
        // The descriptor rides the engine's Hello/Welcome features (data only).
        guard let descriptor = HandoffDescriptor.parse(features: try wire.serverNegotiate(features: [])) else {
            destination.error = "descriptor missing/malformed — refusing (§3.8)"
            return
        }
        print("dest:  descriptor session=\(descriptor.session.prefix(8))… cut=\(descriptor.cut) evid=\(descriptor.evidenceClass)")
        let outcomeJSON = try dstEngine.importCut(session: wire,
                                                  remoteSession: descriptor.session,
                                                  remoteCut: descriptor.cut)
        // CORE-004 app-level gate: declared == measured, or refuse.
        guard try dstEngine.hasObject(hex: descriptor.contentHash),
              IntegrityGate.verify(bytes: try dstEngine.readObject(hex: descriptor.contentHash),
                                   declaredHash: descriptor.contentHash) else {
            destination.error = "declared hash not present/measured mismatch — REFUSED (CORE-004)"
            return
        }
        destination.verified = true
        let mirrorID = try dstEngine.mirrorSessionID(remoteSession: descriptor.session)
        var mirrorCut: UInt64 = 0
        if let o = try JSONSerialization.jsonObject(with: Data(outcomeJSON.utf8)) as? [String: Any],
           let n = o["mirror_cut"] as? Double { mirrorCut = UInt64(n) }
        // The honest L1 engine record: activation-only (§6.5).
        _ = try dstEngine.executeActionAs(
            cut: CutRef(session: mirrorID, number: mirrorCut),
            class: "document.open",
            params: .object(["uri": .string("content://example/\(descriptor.fileName)")]),
            adapterID: "org.carryon.file")
        destination.opened = true
        wire.teardown()
    } catch {
        destination.error = "\(error)"
    }
}

// ---- source: the pipeline (the same one the "Carry File" screen uses).
let result = try awaitSync {
    try await FileHandoffPipeline().run(
        FileHandoffConfig(fileURL: fileURL, receiverAddr: dstAddr, receiverPin: dstPin,
                          identity: srcIdentity, workRoot: root))
}
try awaitSync { await acceptTask.value }

print("source:  ok=\(result.ok) session=\(result.sessionID) hash=\(result.declaredHash.prefix(16))… evid=\(result.evidenceClass)")
print("dest:    verified=\(destination.verified) opened=\(destination.opened)")
if let err = destination.error { print("dest ERROR: \(err)") }
print("ladder:  L0 file carried + verified — \(destination.verified ? "OK" : "FAILED")")
print("ladder:  L1 opened in a real app — \(destination.opened ? "OK (engine record)" : "FAILED")")
print("disclosure: LOCAL run on ONE DEVICE via loopback TLS 1.3 — not physical cross-device evidence (spec §2/§30)")

if !(destination.verified && destination.opened) { exit(1) }

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
