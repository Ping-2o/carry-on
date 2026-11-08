/*
 * PhotoCraftTests.swift — pure, engine-free tests for the stage-10 PhotoCraft
 * UX surface: the transfer descriptor, the import-outcome parser, the progress
 * model and the live-config validation. The engine-side behavior is covered by
 * the Rust adapter/FFI tests and the live two-app integration harness.
 */

import CarryOnKit
import XCTest

final class PhotoCraftTests: XCTestCase {
    // A minimal workspace JSON in the shape the adapter serializes.
    private let workspace = #"{"active_document_index":0,"selected_layers":[],"viewport":{"zoom":1.5,"center":[10.0,20.0],"rotation":3.0},"active_tool":"brush","panels":{}}"#

    func testDescriptorRoundTripAndParse() {
        let d = PhotoCraftDescriptor(session: "sess-1", cut: 3, objects: 51,
                                     name: "photo.pcraft", dirty: true,
                                     workspaceJSON: workspace)
        let parsed = PhotoCraftDescriptor.parse(features: d.features)
        XCTAssertEqual(parsed, d)
        XCTAssertTrue(PhotoCraftDescriptor.isDone(features: [PhotoCraftDescriptor.done]))
        XCTAssertFalse(PhotoCraftDescriptor.isDone(features: d.features))
    }

    func testDescriptorFailsClosed() {
        // No marker at all.
        XCTAssertNil(PhotoCraftDescriptor.parse(features: ["session=s", "cut=1", "objects=2"]))
        // Missing workspace.
        XCTAssertNil(PhotoCraftDescriptor.parse(features: [
            PhotoCraftDescriptor.marker, "session=s", "cut=1", "objects=2",
            "name=n", "dirty=false",
        ]))
        // Non-JSON workspace.
        XCTAssertNil(PhotoCraftDescriptor.parse(features: [
            PhotoCraftDescriptor.marker, "session=s", "cut=1", "objects=2",
            "name=n", "dirty=false", "workspace=not-json",
        ]))
        // Zero objects is not a carry.
        XCTAssertNil(PhotoCraftDescriptor.parse(features: [
            PhotoCraftDescriptor.marker, "session=s", "cut=1", "objects=0",
            "name=n", "dirty=false", "workspace={}",
        ]))
    }

    func testImportOutcomeParsing() {
        XCTAssertEqual(PhotoCraftImportOutcome.parse(#"{"completed":true,"mirror_cut":0}"#),
                       .completed(mirrorCut: 0))
        XCTAssertEqual(
            PhotoCraftImportOutcome.parse(#"{"partial_budget":true,"published":[{"object_id":"a","generation":1}],"deferred":["b","c"],"deferred_bytes":42}"#),
            .partial(published: 1, deferred: 2, deferredBytes: 42))
        XCTAssertEqual(PhotoCraftImportOutcome.parse(#"{"suspended":true,"resume_token":{}}"#), .suspended)
        XCTAssertNil(PhotoCraftImportOutcome.parse("not json"))
        XCTAssertNil(PhotoCraftImportOutcome.parse(#"{"something":true}"#))
    }

    func testProgressFractionClamps() {
        let p = PhotoCraftProgress(totalObjects: 51, publishedObjects: 5, windows: 1,
                                   wireBytes: 360_000, windowBytes: 262_144, widenedWindows: 0)
        XCTAssertEqual(p.fraction, 5.0 / 51.0, accuracy: 1e-9)
        let full = PhotoCraftProgress(totalObjects: 10, publishedObjects: 12, windows: 2,
                                      wireBytes: 1, windowBytes: 1, widenedWindows: 0)
        XCTAssertEqual(full.fraction, 1.0)
        let empty = PhotoCraftProgress(totalObjects: 0, publishedObjects: 0, windows: 0,
                                       wireBytes: 0, windowBytes: 0, widenedWindows: 0)
        XCTAssertEqual(empty.fraction, 0.0)
    }

    func testLiveConfigValidation() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("pcraft-swift-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let token = dir.appendingPathComponent("token")
        try Data("t".utf8).write(to: token)

        let good = PhotoCraftLiveConfig(
            addr: "127.0.0.1:7898", tokenFile: token.path,
            readRoot: dir.path, writeRoot: dir.path)
        XCTAssertTrue(good.isValid)

        // The control channel is loopback-only; a LAN address is refused.
        var lan = good
        lan.addr = "192.168.1.5:7898"
        XCTAssertFalse(lan.isValid)

        var noToken = good
        noToken.tokenFile = dir.appendingPathComponent("missing").path
        XCTAssertFalse(noToken.isValid)
    }

    func testBudgetJSONCarriesTheWindow() {
        let json = PhotoCraftBudget.json(totalBytes: 262_144)
        XCTAssertTrue(json.contains("\"total_net_bytes\":262144"))
        XCTAssertTrue(json.contains("\"battery_thermal\":\"Unrestricted\""))
    }
}
