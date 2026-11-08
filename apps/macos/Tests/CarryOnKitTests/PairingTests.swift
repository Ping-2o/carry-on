/*
 * PairingTests.swift — stage-5 §18.4 surface through the REAL C ABI:
 *
 *   - QR payload format/parse round-trip + fail-closed negatives;
 *   - the six-digit pairing code + the order-independent verification string
 *     (§18.4.4) + the swapped-cert (MITM) negative;
 *   - the provisional pairing channel over loopback: both sides read the
 *     other's TRUE pin, negotiation flows, CONTENT is refused (§18.5);
 *   - §18.4.9 negative: a REVOKED peer's reconnect is refused, fail closed.
 *
 * Labels (§2 law): OBSERVED (LOCAL) — loopback TLS 1.3 on one machine. The
 * physical mac ⇄ A04 run is recorded in the evidence bundle; no platform is
 * "supported" (§30 unrun).
 */

import Foundation
import XCTest

@testable import CarryOnKit

final class PairingTests: XCTestCase {

    // ---- QR payload (§18.4.1-2) --------------------------------------------

    func testPayloadRoundTripAndFailClosed() throws {
        let pin = try DeviceIdentity.generate(name: "dst").pinHex()
        let uri = try formatPairingPayload(deviceName: "Galaxy A04",
                                           addr: "192.168.3.14:54321",
                                           pinHex: pin, code: "428913")
        XCTAssertTrue(uri.hasPrefix("carryon://pair?v=1&"))
        let parsed = try parsePairingPayload(uri)
        XCTAssertEqual(parsed.pinHex, pin)
        XCTAssertEqual(parsed.addr, "192.168.3.14:54321")
        XCTAssertEqual(parsed.code, "428913")
        XCTAssertEqual(parsed.version, 1)

        // Fail-closed negatives.
        XCTAssertThrowsError(try parsePairingPayload("https://pair?v=1"))
        XCTAssertThrowsError(try parsePairingPayload("carryon://pair?v=2&n=x"))
        XCTAssertThrowsError(try parsePairingPayload("carryon://pair?v=1&n=x&a=1.2.3.4:80&k=abc&r=123456"))
        XCTAssertThrowsError(try formatPairingPayload(deviceName: "x", addr: "host:80",
                                                      pinHex: pin, code: "123456"))
    }

    // ---- code + verification string (§18.4.1-2/§18.4.4) ---------------------

    func testVerificationStringOrderIndependentAndMITMDetectable() throws {
        let a = try DeviceIdentity.generate(name: "a")
        let b = try DeviceIdentity.generate(name: "b")
        let eve = try DeviceIdentity.generate(name: "eve")
        let vs = try pairingVerificationString(a.pinHex(), b.pinHex())
        XCTAssertEqual(vs, try pairingVerificationString(b.pinHex(), a.pinHex()))
        XCTAssertEqual(vs.count, 6)
        // A swapped cert changes the string — both devices show a DIFFERENT
        // one and the user's comparison catches the MITM (§18.4.4-5).
        XCTAssertNotEqual(vs, try pairingVerificationString(a.pinHex(), eve.pinHex()))

        // The six-digit code: digits, deterministic per seed.
        let code = try pairingCode(pinHex: a.pinHex(), seed: 48961)
        XCTAssertEqual(code.count, 6)
        XCTAssertTrue(code.allSatisfy(\.isNumber))
        XCTAssertNotEqual(code, try pairingCode(pinHex: a.pinHex(), seed: 48962))
    }

    // ---- provisional pairing channel (§18.4.3) ------------------------------

    func testProvisionalChannelPinsRefusesContentAndRevocationBlocks() throws {
        let src = try DeviceIdentity.generate(name: "carryon-mac-loopback")
        let dst = try DeviceIdentity.generate(name: "a04-loopback")

        let listener = try NetListener.bind("127.0.0.1:0")
        let addr = try listener.addr()

        // Destination accepts provisionally (blocking in its thread).
        let dstHandle = SyncBox<Result<NetSession, Error>?>(nil)
        let dstThread = Thread {
            do {
                let wire = try NetSession.acceptPairing(listener: listener, identity: dst)
                dstHandle.set(.success(wire))
            } catch {
                dstHandle.set(.failure(error))
            }
        }
        dstThread.start()

        let client = try NetSession.connectPairing(addr: addr, identity: src)
        XCTAssertTrue(try client.isPairing())

        // Both sides read the TRUE pin of the other (recorded, not trusted).
        XCTAssertEqual(try client.peerPinHex(), try dst.pinHex())

        // The destination thread completes its accept asynchronously — wait
        // for it (bounded; a handshake failure surfaces as the failure case).
        let deadline = Date().addingTimeInterval(10)
        while dstHandle.value == nil && Date() < deadline {
            Thread.sleep(forTimeInterval: 0.05)
        }

        // Negotiation (pairing traffic) flows: source Hello → destination Welcome.
        let dstWire: NetSession
        switch dstHandle.value {
        case .success(let wire):
            dstWire = wire
        case .failure(let e):
            XCTFail("destination half failed: \(e)")
            return
        default:
            XCTFail("destination half did not produce a session")
            return
        }
        XCTAssertEqual(try dstWire.peerPinHex(), try src.pinHex())
        XCTAssertTrue(try dstWire.isPairing())

        // §18.5: CONTENT on the provisional channel is refused closed, on the
        // client side AND on the server side (engine guard: serve/import on a
        // pairing session fail BEFORE any wire I/O).
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("carryon-pairing-test-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        let engine = try Engine.open(dataDir: tmp.path)
        defer { engine.close() }
        XCTAssertThrowsError(try engine.serveCut(session: client)) { err in
            XCTAssertTrue("\(err)".contains("pairing"), "guard reason expected, got: \(err)")
        }
        XCTAssertThrowsError(try engine.importCut(session: dstWire, remoteSession: "s", remoteCut: 1)) { err in
            XCTAssertTrue("\(err)".contains("pairing"), "guard reason expected, got: \(err)")
        }
        client.teardown()
        dstWire.teardown()

        // ---- §18.4.9: revoke the source; its pinned reconnect is refused.
        let dstTrust = try TrustStore.fromJSON("{\"peers\":{}}")
        try dstTrust.pair(name: "src", pinHex: src.pinHex(), utc: "t")
        XCTAssertTrue(try dstTrust.revoke(pinHex: src.pinHex()))
        XCTAssertFalse(try dstTrust.isTrusted(pinHex: src.pinHex()))

        let listener2 = try NetListener.bind("127.0.0.1:0")
        let addr2 = try listener2.addr()
        let srvResult = SyncBox<Result<Bool, Error>?>(nil)
        let srvThread = Thread {
            do {
                if let wire = try? NetSession.accept(listener: listener2, identity: dst, trust: dstTrust) {
                    // Force I/O: the rejection surfaces and the session drops.
                    let ok = (try? wire.serverNegotiate(features: [])) != nil
                    wire.teardown()
                    srvResult.set(.success(ok))
                } else {
                    srvResult.set(.success(false))
                }
            } catch {
                srvResult.set(.failure(error))
            }
        }
        srvThread.start()

        if let refused = try? NetSession.connect(addr: addr2, identity: src, trust: dstTrust) {
            let ok = (try? refused.clientNegotiate(features: [])) != nil
            refused.teardown()
            XCTAssertFalse(ok, "client I/O against a revoked-peer rejection must fail")
        } // else: connect itself refused — equally fail closed
        while !srvThread.isFinished { Thread.sleep(forTimeInterval: 0.05) }
        switch srvResult.value {
        case .success(let ok):
            XCTAssertFalse(ok, "server must reject the revoked client")
        case .failure(let e):
            XCTFail("server half failed: \(e)")
        default:
            XCTFail("server half did not finish")
        }
    }
}

private final class SyncBox<T> {
    private let lock = NSLock()
    private var _value: T
    init(_ value: T) { _value = value }
    var value: T {
        lock.lock(); defer { lock.unlock() }
        return _value
    }
    func set(_ v: T) {
        lock.lock(); defer { lock.unlock() }
        _value = v
    }
}
