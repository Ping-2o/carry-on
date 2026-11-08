/*
 * HandoffLoopbackTests.swift — integration test: the FULL L0/L1 file handoff
 * in-process on one machine (loopback TLS 1.3, two independent cores):
 * the FileHandoffPipeline source side against a raw-FFI destination that
 * mirrors the Android Receive flow exactly:
 *
 *   accept → server_negotiate (captures the Hello descriptor) → import_cut →
 *   has_object_hex(declared) → read_object → IntegrityGate (measured sha256 ==
 *   declared) → execute_action_as(document.open) → export + verify evidence.
 *
 * Claim label (§2 law): OBSERVED (LOCAL) — one machine, loopback. This test
 * proves the pipeline + descriptor channel + integrity gate wiring, never a
 * physical transfer.
 */

import CCarryon
import Foundation
import XCTest

@testable import CarryOnKit

final class HandoffLoopbackTests: XCTestCase {

    /// Drive one full loopback handoff. `tamper` runs the integrity-negative
    /// path: the source declares a tampered hash and the destination gate must
    /// REFUSE (fail closed) — the CORE-004 visible-failure guarantee.
    private func runLoopback(tamper: Bool) throws -> (served: Bool, verified: Bool, refused: Bool) {
        let root = NSTemporaryDirectory() + "carryon-handoff-loopback-\(UUID().uuidString)"
        try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: root) }

        // The real carried file.
        let fileURL = URL(fileURLWithPath: root + "/carryon-doc.png")
        let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) + Data((0..<64).map { $0 })
        try png.write(to: fileURL)

        // ---- destination: core, file adapter (placeholder path, as the
        // reference shell does), identity + trust, listener
        let dstDir = root + "/dest"
        let placeholder = "receiver store placeholder"
        try placeholder.write(toFile: root + "/placeholder.txt", atomically: true, encoding: .utf8)
        let dstEngine = try Engine.open(dataDir: dstDir)
        _ = try dstEngine.registerAdapter(id: "org.carryon.file",
                                          paramsJSON: "{\"path\":\"\(root + "/placeholder.txt")\"}")

        let dstIdentity = try DeviceIdentity.generate(name: "loopback-dest")
        let dstPin = try dstIdentity.pinHex()
        // Pin the SOURCE first (mutual pinning, §18.5): the source identity is
        // pre-generated and passed to the pipeline, exactly as the macOS UI
        // pre-arms its identity so the receiver can pin it.
        let srcIdentity = try DeviceIdentity.generate(name: "loopback-src")
        let srcPin = try srcIdentity.pinHex()
        let dstTrust = try TrustStore.fromJSON("{\"peers\":{}}")
        try dstTrust.pair(name: "source", pinHex: srcPin,
                          utc: ISO8601DateFormatter().string(from: Date()))

        let listener = try NetListener.bind("127.0.0.1:0")
        let dstAddr = try listener.addr()

        let dstResultBox = ResultBox()
        let acceptTask = Task.detached {
            do {
                let sess = try NetSession.accept(listener: listener, identity: dstIdentity, trust: dstTrust)
                let theirs = try sess.serverNegotiate(features: [])
                let descriptor = try XCTUnwrap(HandoffDescriptor.parse(features: theirs))
                dstResultBox.descriptor = descriptor
                // Destination-controlled chunking (§18.6): small so the
                // two-phase publish is visible in the counters.
                try dstEngine.setChunkSize(HandoffValidation.clampChunkSize(65_536))
                let outcomeJSON = try dstEngine.importCut(session: sess,
                                                          remoteSession: descriptor.session,
                                                          remoteCut: descriptor.cut)
                guard let v = parseAnyJSON(outcomeJSON), case .object(let o) = v,
                      o["completed"]?.boolValue == true else {
                    dstResultBox.error = "unexpected import outcome: \(outcomeJSON)"
                    return
                }
                // ---- CORE-004 app-level gate: declared == measured, or refuse
                let present = try dstEngine.hasObject(hex: descriptor.contentHash)
                var verified = false
                var refused = false
                if present {
                    let bytes = try dstEngine.readObject(hex: descriptor.contentHash)
                    if IntegrityGate.verify(bytes: bytes, declaredHash: descriptor.contentHash) {
                        verified = true
                    } else {
                        refused = true // measured ≠ declared: fail closed
                    }
                } else {
                    refused = true // declared object absent: fail closed
                }
                dstResultBox.verified = verified
                dstResultBox.refused = refused
                if verified {
                    // The honest L1 engine record: activation-only (§6.5
                    // OracleKind::None), through the adapter boundary.
                    let mirrorID = try dstEngine.mirrorSessionID(remoteSession: descriptor.session)
                    let mirrorCut = o["mirror_cut"]?.numberValue.map { UInt64($0) } ?? 0
                    _ = try dstEngine.executeActionAs(
                        cut: CutRef(session: mirrorID, number: mirrorCut),
                        class: "document.open",
                        params: .object(["uri": .string("content://loopback/\(descriptor.fileName)")]),
                        adapterID: "org.carryon.file")
                }
                dstResultBox.done = true
                sess.teardown()
            } catch {
                dstResultBox.error = "\(error)"
            }
        }

        // ---- source: the SAME pipeline the macOS app screen + smoke binary use
        let config = FileHandoffConfig(fileURL: fileURL, receiverAddr: dstAddr,
                                       receiverPin: dstPin,
                                       identity: srcIdentity,
                                       integrityNegative: tamper,
                                       workRoot: root)
        var served = false
        var verifyOK = false
        let result = try awaitSync {
            try await FileHandoffPipeline().run(config)
        }
        served = result.ok
        verifyOK = result.verifyOK

        // Wait for the destination side to settle.
        for _ in 0..<100 where !dstResultBox.done && dstResultBox.error == nil {
            Thread.sleep(forTimeInterval: 0.05)
        }
        acceptTask.cancel()

        XCTAssertEqual(dstResultBox.error, nil, "destination side failed: \(dstResultBox.error ?? "")")
        if !tamper {
            XCTAssertTrue(served, "source evidence did not verify")
            XCTAssertTrue(verifyOK)
            XCTAssertTrue(dstResultBox.verified, "destination gate must verify the real bytes")
            XCTAssertFalse(dstResultBox.refused)
            let d = try XCTUnwrap(dstResultBox.descriptor)
            XCTAssertEqual(d.evidenceClass, "local") // loopback → honest LOCAL label
        } else {
            XCTAssertTrue(served, "the engine still carries + verifies the real bytes")
            XCTAssertTrue(dstResultBox.refused, "tampered declared hash must be REFUSED")
            XCTAssertFalse(dstResultBox.verified, "no success on a mismatched object (CORE-004)")
        }
        return (served, verifyOK, dstResultBox.refused)
    }

    func testLoopbackPositive() throws {
        _ = try runLoopback(tamper: false)
    }

    func testLoopbackNegativeTamperedHash() throws {
        _ = try runLoopback(tamper: true)
    }
}

/// Tiny thread-safe box for the detached destination task's outcomes.
final class ResultBox: @unchecked Sendable {
    private let lock = NSLock()
    var _descriptor: HandoffDescriptor?
    var _verified = false
    var _refused = false
    var _done = false
    var _error: String?

    var descriptor: HandoffDescriptor? {
        get { lock.lock(); defer { lock.unlock() }; return _descriptor }
        set { lock.lock(); defer { lock.unlock() }; _descriptor = newValue }
    }
    var verified: Bool {
        get { lock.lock(); defer { lock.unlock() }; return _verified }
        set { lock.lock(); defer { lock.unlock() }; _verified = newValue }
    }
    var refused: Bool {
        get { lock.lock(); defer { lock.unlock() }; return _refused }
        set { lock.lock(); defer { lock.unlock() }; _refused = newValue }
    }
    var done: Bool {
        get { lock.lock(); defer { lock.unlock() }; return _done }
        set { lock.lock(); defer { lock.unlock() }; _done = newValue }
    }
    var error: String? {
        get { lock.lock(); defer { lock.unlock() }; return _error }
        set { lock.lock(); defer { lock.unlock() }; _error = newValue }
    }
}

/// Thread-safe result box for awaiting an async body synchronously in tests
/// (Swift forbids local classes nested in generic functions).
private final class SyncBox<T>: @unchecked Sendable {
    var value: T?
    var error: Error?
}

private func awaitSync<T>(_ body: @escaping () async throws -> T) throws -> T {
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
