/*
 * EngineDiscoveryContractTests.swift — unit tests for the ENGINE-owned §18.3
 * TXT contract through the C ABI (stage 4): encode produces exactly the five
 * canonical keys, validation round-trips, foreign keys are IGNORED (counted,
 * never interpreted), and a malformed record FAILS CLOSED (the shell must drop
 * the peer — it never repairs it).
 *
 * Local, headless tests: they exercise the engine contract over the FFI, not
 * any network claim (§2 — the live multicast run is owner-machine evidence).
 */

import CCarryon
import Foundation
import XCTest

@testable import CarryOnKit

final class EngineDiscoveryContractTests: XCTestCase {

    private func fields(deviceClass: String = "mobile") -> TxtFields {
        TxtFields(protocolMajor: 1, instanceID: "ab12cd34", pairingRequired: true,
                  deviceClass: deviceClass, connectionHint: "54321")
    }

    // ---- encode: canonical construction --------------------------------------

    func testEncodeProducesExactlyTheFiveSection1833KeysInOrder() throws {
        let pairsJSON = try EngineDiscoveryContract.encodeTxt(fields: fields())
        let pairs = try XCTUnwrap(
            (try JSONSerialization.jsonObject(with: Data(pairsJSON.utf8))) as? [[String]])
        let keys = pairs.map { $0[0] }
        XCTAssertEqual(keys, ["pm", "id", "pr", "dc", "ch"], "§18.3 minimal set, canonical order")
        let values = pairs.map { $0[1] }
        XCTAssertEqual(values, ["1", "ab12cd34", "1", "mobile", "54321"])
        // Construction minimality: nothing a pilot could leak is present.
        let blob = pairsJSON
        for forbidden in ["user", "session", "sha256", "pubkey", "BEGIN", "title"] {
            XCTAssertFalse(blob.contains(forbidden), "TXT leaked \(forbidden)")
        }
    }

    func testEncodeRefusesBadDeviceClass() {
        // A model string is NOT a device class — the engine refuses (fail closed).
        XCTAssertThrowsError(
            try EngineDiscoveryContract.encodeTxt(fields: fields(deviceClass: "iPhone 14")))
    }

    // ---- validate: fail-closed receive filter ---------------------------------

    func testValidateRoundTripAndForeignKeysIgnored() throws {
        let pairsJSON = try EngineDiscoveryContract.encodeTxt(fields: fields())
        var pairs = try XCTUnwrap(
            (try JSONSerialization.jsonObject(with: Data(pairsJSON.utf8))) as? [[String]])
        // A hostile record carries foreign keys; the engine ignores them.
        pairs.append(["vendor_key", "opaque"])
        pairs.append(["user", "alice"])
        let json = try String(
            decoding: JSONEncoder().encode(pairs.map { [$0[0], $0[1]] }), as: UTF8.self)
        let validated = try EngineDiscoveryContract.validateTxtPairs(json)
        XCTAssertEqual(validated.fields.protocol_major, 1)
        XCTAssertEqual(validated.fields.instance_id, "ab12cd34")
        XCTAssertEqual(validated.fields.device_class, "mobile")
        XCTAssertTrue(validated.fields.pairing_required)
        XCTAssertEqual(Set(validated.ignoredKeys), Set(["vendor_key", "user"]))
    }

    func testValidateFailsClosedOnMissingKey() throws {
        let pairsJSON = try EngineDiscoveryContract.encodeTxt(fields: fields())
        var pairs = try XCTUnwrap(
            (try JSONSerialization.jsonObject(with: Data(pairsJSON.utf8))) as? [[String]])
        pairs.removeFirst() // drop "pm" — the peer must be dropped, never guessed
        let json = try String(
            decoding: JSONEncoder().encode(pairs.map { [$0[0], $0[1]] }), as: UTF8.self)
        XCTAssertThrowsError(try EngineDiscoveryContract.validateTxtPairs(json))
    }

    func testValidateAcceptsBothPrEncodings() throws {
        for (encoded, expected) in [("1", true), ("true", true), ("0", false), ("false", false)] {
            let pairsJSON = try EngineDiscoveryContract.encodeTxt(
                fields: TxtFields(protocolMajor: 1, instanceID: "ab12cd34",
                                  pairingRequired: expected, deviceClass: "desktop",
                                  connectionHint: "none"))
            var pairs = try XCTUnwrap(
                (try JSONSerialization.jsonObject(with: Data(pairsJSON.utf8))) as? [[String]])
            pairs[2] = ["pr", encoded] // the pilots' two encodings, §18.3 reality
            let json = try String(
                decoding: JSONEncoder().encode(pairs.map { [$0[0], $0[1]] }), as: UTF8.self)
            let validated = try EngineDiscoveryContract.validateTxtPairs(json)
            XCTAssertEqual(validated.fields.pairing_required, expected, "pr=\(encoded)")
        }
    }
}

private extension EngineDiscoveryContract {
    /// Pairs as raw `[[key,value],…]` JSON — the exact shape the engine ABI takes.
    static func validateTxtPairs(_ json: String) throws
        -> (fields: TxtFields, ignoredKeys: [String]) {
        let reply = try outString("carryon_discovery_txt_validate") { out in
            json.withCString { carryon_discovery_txt_validate($0, out) }
        }
        struct Validated: Codable {
            var txt: TxtFields
            var ignored_keys: [String]
        }
        return try { v in (v.txt, v.ignored_keys) }(JSONDecoder().decode(Validated.self,
                                                                         from: Data(reply.utf8)))
    }
}
