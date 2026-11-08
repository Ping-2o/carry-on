// swift-tools-version:5.9
// Carry-On for macOS — native SwiftUI wrapper over the real Carry-On engine C ABI
// (crates/carryon-ffi/include/carryon.h, ABI 1.1). Swift 5 language mode on the
// Swift 6.4.2 compiler; deployment target macOS 14.0 for @Observable.
//
// Targets:
//   CCarryon           C target carrying the vendored carryon.h; links the engine
//                      dylib from Vendor/ (never committed; Scripts/build_engine.sh).
//   CarryOnKit         Swift binding (core/net/evidence/demo pipeline/discovery).
//   CarryOnApp         SwiftUI executable.
//   carryon-demo-smoke headless verification executable (exits 0 iff evidence
//                      verifies ok:true).
import PackageDescription
import Foundation

// Absolute path of Vendor/ (Package.swift's directory), so dev builds resolve
// libcarryon_ffi without environment hacks; the app bundle additionally gets
// @executable_path/../Frameworks from Scripts/make_app.sh.
let manifestDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().standardized.path
let vendorDir = manifestDir + "/Vendor"

let package = Package(
    name: "CarryOn",
    platforms: [.macOS(.v14)],
    targets: [
        .target(
            name: "CCarryon",
            path: "Sources/CCarryon",
            publicHeadersPath: "include",
            linkerSettings: [
                // unsafeFlags: acceptable for a root package (plan.md). -L finds
                // libcarryon_ffi.dylib; the rpath makes `swift run` work from .build/.
                .unsafeFlags(["-L", vendorDir]),
                .linkedLibrary("carryon_ffi"),
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", vendorDir]),
            ]
        ),
        .target(name: "CarryOnKit", dependencies: ["CCarryon"]),
        .executableTarget(
            name: "CarryOnApp",
            dependencies: ["CarryOnKit"]
        ),
        .executableTarget(
            name: "carryon-demo-smoke",
            dependencies: ["CarryOnKit"]
        ),
        .executableTarget(
            name: "carryon-filehandoff-smoke",
            dependencies: ["CarryOnKit"]
        ),
        .testTarget(
            name: "CarryOnKitTests",
            dependencies: ["CarryOnKit", "CCarryon"]
        ),
    ]
)
