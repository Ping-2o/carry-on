/*
 * HandoffTests.swift — unit tests for the L0/L1 file handoff's pure logic:
 * configuration validation (§18.6 chunk clamp — the stage-1 device bug class),
 * IPv4:port receiver-address validation, pin validation, the §3.8-safe
 * descriptor encode/parse round-trip (fail closed on malformed input), the
 * CORE-004 integrity gate, and the mime map.
 *
 * These are local, pure-Swift tests: they prove the VALIDATION LOGIC, not any
 * transport claim (labels follow §2 — see the pipeline + evidence for those).
 */

import Foundation
import XCTest

@testable import CarryOnKit

final class HandoffTests: XCTestCase {

    // ---- §18.6 chunk clamp (stage-1 bug regression class) --------------------

    func testChunkClampBounds() {
        // The engine admits [4096, 8 MiB]; the knob clamps, never fails OPEN.
        XCTAssertEqual(HandoffValidation.clampChunkSize(0), 4096)
        XCTAssertEqual(HandoffValidation.clampChunkSize(256), 4096) // the stage-1 device bug value
        XCTAssertEqual(HandoffValidation.clampChunkSize(4095), 4096)
        XCTAssertEqual(HandoffValidation.clampChunkSize(4096), 4096)
        XCTAssertEqual(HandoffValidation.clampChunkSize(65_536), 65_536)
        XCTAssertEqual(HandoffValidation.clampChunkSize(8_388_608), 8_388_608)
        XCTAssertEqual(HandoffValidation.clampChunkSize(8_388_609), 8_388_608)
        XCTAssertEqual(HandoffValidation.clampChunkSize(UInt64.max), 8_388_608)
    }

    // ---- receiver address validation (manual IP, pre-discovery) --------------

    func testReceiverAddrAcceptsIPv4Port() {
        XCTAssertTrue(HandoffValidation.isValidReceiverAddr("192.168.3.50:48961"))
        XCTAssertTrue(HandoffValidation.isValidReceiverAddr("10.0.0.1:1"))
        XCTAssertTrue(HandoffValidation.isValidReceiverAddr("255.255.255.255:65535"))
        XCTAssertTrue(HandoffValidation.isValidReceiverAddr("127.0.0.1:48961")) // loopback is valid — it is the LOCAL evidence class
    }

    func testReceiverAddrRejectsMalformed() {
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("")) // empty
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50")) // no port
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:")) // empty port
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:0")) // port 0
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:65536")) // port overflow
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:abc")) // non-numeric
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50:007")) // leading zeros: strict form
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("256.168.3.50:48961")) // octet overflow
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3:48961")) // 3 octets
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("192.168.3.50.1:48961")) // 5 octets
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("a.b.c.d:80")) // non-numeric octets
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("host:80")) // hostname: manual mode is IPv4 until discovery
        XCTAssertFalse(HandoffValidation.isValidReceiverAddr("::1:48961")) // IPv6: out of scope
    }

    // ---- pin validation --------------------------------------------------------

    func testPinValidation() {
        let good = String(repeating: "0123456789abcdef", count: 4)
        XCTAssertTrue(HandoffValidation.isValidPinHex(good))
        XCTAssertFalse(HandoffValidation.isValidPinHex(good.uppercased())) // lowercase only, strict
        XCTAssertFalse(HandoffValidation.isValidPinHex(String(repeating: "ab", count: 31))) // 62
        XCTAssertFalse(HandoffValidation.isValidPinHex(good + "g")) // non-hex
        XCTAssertFalse(HandoffValidation.isValidPinHex(""))
    }

    // ---- descriptor: §3.8-safe encode/parse ------------------------------------

    func testDescriptorRoundTrip() {
        let d = HandoffDescriptor(session: "11111111-2222-3333-4444-555555555555",
                                  cut: 7,
                                  contentHash: String(repeating: "ab", count: 32),
                                  fileName: "carryon-doc.png",
                                  mimeType: "image/png",
                                  evidenceClass: "physical")
        let parsed = HandoffDescriptor.parse(features: d.features)
        XCTAssertEqual(parsed, d)
    }

    func testDescriptorFailsClosedOnMalformed() {
        let d = HandoffDescriptor(session: "s", cut: 1,
                                  contentHash: String(repeating: "ab", count: 32),
                                  fileName: "f.png", mimeType: "image/png",
                                  evidenceClass: "local")
        XCTAssertNil(HandoffDescriptor.parse(features: [])) // no marker
        XCTAssertNil(HandoffDescriptor.parse(features: ["org.carryon.handoff.v1"])) // marker only
        // Missing hash → refuse (a destination never guesses its import params).
        XCTAssertNil(HandoffDescriptor.parse(features: d.features.filter { !$0.hasPrefix("hash=") }))
        // Malformed hash → refuse.
        XCTAssertNil(HandoffDescriptor.parse(features: d.features.map {
            $0.hasPrefix("hash=") ? "hash=zz" : $0
        }))
        // Bad evidence class → refuse (the label is honest or absent).
        XCTAssertNil(HandoffDescriptor.parse(features: d.features.map {
            $0.hasPrefix("evid=") ? "evid=sure" : $0
        }))
        // Unknown extra features are ignored, never interpreted (§18.3 style).
        var withExtras = d.features
        withExtras.append("exec=/bin/sh") // hostile probe: must be IGNORED
        XCTAssertEqual(HandoffDescriptor.parse(features: withExtras), d)
    }

    // ---- CORE-004 integrity gate ------------------------------------------------

    func testIntegrityGate() {
        let bytes = Data("carry-on file handoff bytes".utf8)
        let declared = HandoffValidation.sha256Hex(of: bytes)
        XCTAssertTrue(IntegrityGate.verify(bytes: bytes, declaredHash: declared))
        // One flipped byte → refuse (fail closed, the UI shows FAIL).
        XCTAssertFalse(IntegrityGate.verify(bytes: bytes + Data([0x00]), declaredHash: declared))
        // A tampered DECLARED hash → refuse (the negative-test path).
        XCTAssertFalse(IntegrityGate.verify(bytes: bytes, declaredHash: declared.replacingOccurrences(of: "a", with: "b")))
    }

    // ---- mime map -----------------------------------------------------------------

    func testMimeMap() {
        XCTAssertEqual(HandoffValidation.mime(forExtension: "png"), "image/png")
        XCTAssertEqual(HandoffValidation.mime(forExtension: "PNG"), "image/png")
        XCTAssertEqual(HandoffValidation.mime(forExtension: "jpg"), "image/jpeg")
        XCTAssertEqual(HandoffValidation.mime(forExtension: "pdf"), "application/pdf")
        XCTAssertEqual(HandoffValidation.mime(forExtension: "txt"), "text/plain")
        XCTAssertEqual(HandoffValidation.mime(forExtension: "weird"), "application/octet-stream")
        XCTAssertEqual(HandoffValidation.mime(forExtension: ""), "application/octet-stream")
    }

    // ---- evidence class from the dialled address ------------------------------------

    func testEvidenceClass() {
        XCTAssertEqual(HandoffValidation.evidenceClass(dialing: "127.0.0.1:48961"), "local")
        XCTAssertEqual(HandoffValidation.evidenceClass(dialing: "192.168.3.50:48961"), "physical")
        XCTAssertEqual(HandoffValidation.evidenceClass(dialing: "10.1.2.3:1"), "physical")
    }
}
