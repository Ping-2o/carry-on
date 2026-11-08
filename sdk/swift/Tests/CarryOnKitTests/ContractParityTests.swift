/*
 * ContractParityTests.swift — the SDK contract scenarios. The SAME scenario
 * table runs through the Kotlin SDK (apps/android/carryon-kit
 * src/test/.../ContractParityTest.kt) — these are the parity contract tests:
 *
 *   carry-file/loopback-positive   file → cut → TLS → descriptor → import →
 *                                  CORE-004 gate → document.open, byte-for-byte
 *   carry-file/tampered-refused    declared hash tampered → the destination
 *                                  REFUSES (fail closed), no success reported
 *   demo/8-stages-verify           the full demo pipeline, evidence verifies ok
 *   abi/version-gate               SDK version follows ABI major.minor; an
 *                                  engine older than the SDK is refused clearly
 *
 * Labels (§2): loopback = OBSERVED (LOCAL). The physical cross-device run of
 * the same ladder lives in apps/android/evidence/20261108-a04-l0l1/.
 */

import Foundation
import XCTest

@testable import CarryOnKit

final class ContractParityTests: XCTestCase {

    // ---- carry-file/loopback-positive ---------------------------------------

    func testCarryFileLoopbackPositive() throws {
        try CarryOnSDK.requireCompatibleEngine()
        let outcome = try runCarryFile(tamper: false)
        XCTAssertTrue(outcome.verified, "destination must verify measured sha256 == declared")
        XCTAssertFalse(outcome.refused)
        XCTAssertTrue(outcome.opened, "engine document.open activation record must run")
        XCTAssertEqual(outcome.evidenceClass, "local")
    }

    // ---- carry-file/tampered-refused ----------------------------------------

    func testCarryFileTamperedRefused() throws {
        let outcome = try runCarryFile(tamper: true)
        XCTAssertFalse(outcome.verified, "no success may be reported on a mismatched object")
        XCTAssertTrue(outcome.refused, "tampered declared hash must be REFUSED (CORE-004)")
        XCTAssertFalse(outcome.opened)
    }

    // ---- demo/8-stages-verify ------------------------------------------------

    func testDemoParityVerifies() async throws {
        // The same scenario the Kotlin SDK's demo-parity check asserts the
        // shape of: 8 stages with the evidence bundle verifying ok.
        let result = try await DemoPipeline().run(DemoConfig())
        XCTAssertTrue(result.verifyOK, "evidence bundle must verify: \(result.verifyMessage)")
        XCTAssertEqual(result.stageOutcomes.count, DemoStage.allCases.count)
        XCTAssertNil(result.failure)
    }

    // ---- abi/version-gate ----------------------------------------------------

    func testAbiVersionGateIsEnforced() throws {
        try CarryOnSDK.requireCompatibleEngine()
        XCTAssertEqual(CarryOnSDK.version.split(separator: ".").count, 3)
    }

    // ---- shared runner (mirrors the Kotlin ContractParityTest) ---------------

    private struct Outcome {
        var verified = false
        var refused = false
        var opened = false
        var evidenceClass = ""
    }

    private func runCarryFile(tamper: Bool) throws -> Outcome {
        let root = NSTemporaryDirectory() + "carryon-sdk-contract-\(UUID().uuidString)"
        try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: root) }

        let fileURL = URL(fileURLWithPath: root + "/carryon-doc.png")
        try Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] + (0..<256).map { UInt8($0 % 251) })
            .write(to: fileURL)

        // destination side (the Receive screen's steps)
        try "placeholder".write(toFile: root + "/placeholder.txt", atomically: true, encoding: .utf8)
        let dstEngine = try Engine.open(dataDir: root + "/dest")
        _ = try dstEngine.registerAdapter(id: "org.carryon.file",
                                          paramsJSON: "{\"path\":\"\(root + "/placeholder.txt")\"}")
        let dstIdentity = try DeviceIdentity.generate(name: "contract-dest")
        let dstPin = try dstIdentity.pinHex()
        let srcIdentity = try DeviceIdentity.generate(name: "contract-src")
        let dstTrust = try TrustStore.fromJSON("{\"peers\":{}}")
        try dstTrust.pair(name: "source", pinHex: try srcIdentity.pinHex(),
                          utc: ISO8601DateFormatter().string(from: Date()))
        let listener = try NetListener.bind("127.0.0.1:0")
        let dstAddr = try listener.addr()

        final class Box: @unchecked Sendable { var out = Outcome(); var error: String? }
        let box = Box()
        let acceptTask = Task.detached {
            do {
                let wire = try NetSession.accept(listener: listener, identity: dstIdentity, trust: dstTrust)
                guard let d = HandoffDescriptor.parse(features: try wire.serverNegotiate(features: [])) else {
                    box.error = "descriptor malformed"
                    return
                }
                box.out.evidenceClass = d.evidenceClass
                _ = try dstEngine.importCut(session: wire, remoteSession: d.session, remoteCut: d.cut)
                if try dstEngine.hasObject(hex: d.contentHash),
                   IntegrityGate.verify(bytes: try dstEngine.readObject(hex: d.contentHash),
                                        declaredHash: d.contentHash) {
                    box.out.verified = true
                    let mirror = try dstEngine.mirrorSessionID(remoteSession: d.session)
                    _ = try dstEngine.executeActionAs(
                        cut: CutRef(session: mirror, number: 0),
                        class: "document.open",
                        params: .object(["uri": .string("content://contract/\(d.fileName)")]),
                        adapterID: "org.carryon.file")
                    box.out.opened = true
                } else {
                    box.out.refused = true
                }
                wire.teardown()
            } catch {
                box.error = "\(error)"
            }
        }

        let result = try sync {
            try await FileHandoffPipeline().run(
                FileHandoffConfig(fileURL: fileURL, receiverAddr: dstAddr, receiverPin: dstPin,
                                  identity: srcIdentity, integrityNegative: tamper, workRoot: root))
        }
        XCTAssertTrue(result.ok, "source evidence must verify: \(result.verifyMessage)")
        try sync { await acceptTask.value }
        XCTAssertNil(box.error, "destination failed: \(box.error ?? "")")
        return box.out
    }

    private func sync<T>(_ body: @escaping () async throws -> T) throws -> T {
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
}

private final class SyncBox<T>: @unchecked Sendable {
    var value: T?
    var error: Error?
}
