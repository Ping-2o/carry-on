/*
 * AbiCompatibilityTests.swift — the ABI compatibility suite (plan.md stage 9:
 * "extend the existing cross-check"). Three-way:
 *
 *   crates/carryon-ffi/include/carryon.h  (the contract)
 *   Vendor/CarryOnEngine.xcframework      (the archive clients link)
 *   Sources/CarryOnKit + Sources/CCarryon (the bindings)
 *
 * Every `carryon_*` symbol the header declares must (a) be EXPORTED by the
 * vendored archive and (b) be referenced by the binding sources — or appear in
 * the explicit allowlist below with a reason. The ABI version constants must
 * agree between the header and SDK.
 *
 * This is the Swift half of the same suite the Kotlin SDK runs against the
 * shim + bindings (dev.carryon.kit.AbiCompatibilityTest).
 */

import CCarryon
import XCTest

@testable import CarryOnKit

final class AbiCompatibilityTests: XCTestCase {

    /// Header symbols intentionally not referenced by the Swift binding.
    /// (None today: the binding covers the whole append-only surface. Keep the
    /// list explicit so a NEW unbound symbol is a conscious decision.)
    private static let unboundAllowlist: Set<String> = []

    // ---- helpers -------------------------------------------------------------

    private var packageRoot: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // Tests/CarryOnKitTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // package root
    }

    private func headerSymbols() throws -> [String] {
        let header = try String(contentsOf: packageRoot
            .appendingPathComponent("Sources/CCarryon/include/carryon.h"), encoding: .utf8)
        // Lines like: `int32_t carryon_import_cut(...);` / `CarryonCore *carryon_core_open(...)`
        let regex = try NSRegularExpression(pattern: #"\b(carryon_[a-z0-9_]+)\s*\("#, options: [])
        let ns = header as NSString
        var names = Set<String>()
        for m in regex.matches(in: header, range: NSRange(location: 0, length: ns.length)) {
            names.insert(ns.substring(with: m.range(at: 1)))
        }
        return names.sorted()
    }

    private func bindingSources() throws -> String {
        let fm = FileManager.default
        var text = ""
        for dir in ["Sources/CarryOnKit", "Sources/CCarryon"] {
            let url = packageRoot.appendingPathComponent(dir)
            for f in try fm.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)
            where f.pathExtension == "swift" || f.pathExtension == "c" || f.pathExtension == "h" {
                text += (try? String(contentsOf: f, encoding: .utf8)) ?? ""
            }
        }
        return text
    }

    private func archivePath() throws -> String {
        let xcfw = packageRoot.appendingPathComponent("Vendor/CarryOnEngine.xcframework")
        let fm = FileManager.default
        let entries = try fm.contentsOfDirectory(at: xcfw, includingPropertiesForKeys: nil)
        guard let slice = entries.first(where: { $0.lastPathComponent.contains("macos") }) else {
            throw XCTSkip("no macOS slice in the vendored xcframework")
        }
        let arch = try fm.contentsOfDirectory(at: slice, includingPropertiesForKeys: nil)
            .first { $0.pathExtension == "a" }
        guard let arch else { throw XCTSkip("no static archive in the macOS slice") }
        return arch.path
    }

    private func archiveExports() throws -> Set<String> {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/nm")
        p.arguments = ["-gU", try archivePath()]
        let pipe = Pipe()
        let errPipe = Pipe() // LLVM-version bitcode warnings on std members: noise, not failure
        p.standardOutput = pipe
        p.standardError = errPipe
        try p.run()
        // Read to EOF BEFORE waiting: nm's output exceeds the pipe buffer
        // (a wait-then-read order deadlocks on large archives).
        let out = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        _ = errPipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        var names = Set<String>()
        for line in out.split(separator: "\n") {
            // `0000000000000000 T _carryon_core_open`
            guard let last = line.split(separator: " ").last else { continue }
            let symbol = last.hasPrefix("_") ? String(last.dropFirst()) : String(last)
            if symbol.hasPrefix("carryon_") { names.insert(symbol) }
        }
        return names
    }

    // ---- the suite -----------------------------------------------------------

    func testHeaderSymbolsAreExportedByTheArchive() throws {
        let header = Set(try headerSymbols())
        XCTAssertGreaterThan(header.count, 50, "header symbol extraction broke")
        let exports = try archiveExports()
        let missing = header.subtracting(exports)
        XCTAssertTrue(missing.isEmpty,
                      "header declares symbols the engine archive does not export: \(missing.sorted())")
    }

    func testEveryHeaderSymbolIsBoundOrAllowlisted() throws {
        let symbols = try headerSymbols()
        let bindings = try bindingSources()
        let unbound = symbols.filter {
            !bindings.contains($0) && !Self.unboundAllowlist.contains($0)
        }
        XCTAssertTrue(unbound.isEmpty,
                      "header symbols neither referenced by the bindings nor allowlisted: \(unbound)")
    }

    func testHeaderIsTheOneTheEngineWasBuiltAgainst() throws {
        // The vendored header must be byte-identical to the engine crate's
        // (the fetch script drift-checks this at build time; pin it in tests).
        let vendored = try String(contentsOf: packageRoot
            .appendingPathComponent("Sources/CCarryon/include/carryon.h"), encoding: .utf8)
        let crate = try String(contentsOf: packageRoot
            .appendingPathComponent("../../crates/carryon-ffi/include/carryon.h"),
            encoding: .utf8)
        XCTAssertEqual(vendored, crate, "vendored carryon.h drifted from the crate")
    }

    func testSdkVersionFollowsAbiMajorMinor() {
        // Plan.md stage 9: the SDK version follows the ABI major.minor.
        let parts = CarryOnSDK.version.split(separator: ".").compactMap { Int($0) }
        XCTAssertEqual(parts.count, 3, "SDK version must be semver MAJOR.MINOR.PATCH")
        XCTAssertEqual(UInt32(parts[0]), CarryOnSDK.abiMajor)
        XCTAssertEqual(UInt32(parts[1]), CarryOnSDK.abiMinor)
        XCTAssertEqual(CarryOnSDK.abiMajor, CARRYON_ABI_MAJOR)
        XCTAssertEqual(CarryOnSDK.abiMinor, CARRYON_ABI_MINOR)
    }

    func testEngineReportsTheHeadersAbi() throws {
        try CarryOnSDK.requireCompatibleEngine() // the startup gate, exercised
        var major: UInt32 = 0, minor: UInt32 = 0
        carryon_abi_version(&major, &minor)
        XCTAssertEqual(major, CARRYON_ABI_MAJOR)
        XCTAssertGreaterThanOrEqual(minor, CARRYON_ABI_MINOR)
    }

    func testKitSurfaceStaysOutOfTheAppShell() throws {
        // The SDK must not import SwiftUI / reference the app target.
        let bindings = try bindingSources()
        XCTAssertFalse(bindings.contains("import SwiftUI"))
        XCTAssertFalse(bindings.contains("CarryOnApp"))
    }
}
