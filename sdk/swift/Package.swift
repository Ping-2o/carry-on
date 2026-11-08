// swift-tools-version:5.9
// CarryOnKit — the distributable Swift SDK over the real Carry-On engine C ABI
// (crates/carryon-ffi/include/carryon.h, ABI 1.2, append-only within major 1).
//
// Distribution model (plan.md stage 9): SOURCE package + the engine as a
// FETCHED STATIC ARCHIVE. The binding sources are Swift (copied from
// apps/macos by Scripts/sync_from_app.sh — the app package stays the root
// owner); the engine archive is built by Scripts/build_engine_static.sh and
// vendored into Vendor/ (gitignored, never committed — repo law for every
// engine binary). A static archive is a supported SwiftPM binary target and
// links WITHOUT unsafeFlags — this package is a real dependency for clients,
// and no binary in it is ad-hoc-signed (PLAT-006 stays out of scope).
//
// Targets:
//   CarryOnEngine      binaryTarget: Vendor/libcarryon_ffi.a (fetched)
//   CCarryon           vendored carryon.h + module map (the C ABI contract)
//   CarryOnKit         the Swift binding (core/net/evidence/demo/handoff/pairing/discovery)
//   CarryFileExample   example: carry a file end-to-end, in-process loopback
//   DemoExample        example: the 8-stage demo pipeline, in-process loopback
//   CarryOnKitTests    contract tests (parity scenario + ABI compatibility suite)
import PackageDescription

let package = Package(
    name: "CarryOnKit",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "CarryOnKit", targets: ["CarryOnKit"]),
    ],
    targets: [
        .binaryTarget(name: "CarryOnEngine", path: "Vendor/CarryOnEngine.xcframework"),
        .target(
            name: "CCarryon",
            dependencies: ["CarryOnEngine"],
            publicHeadersPath: "include",
            // The Rust static archive needs its platform link inputs; explicit
            // here so clients need no extra build settings.
            linkerSettings: [
                .linkedFramework("Security"),
                .linkedFramework("CoreFoundation"),
                .linkedLibrary("resolv"),
            ]
        ),
        .target(name: "CarryOnKit", dependencies: ["CCarryon"]),
        .executableTarget(
            name: "CarryFileExample",
            dependencies: ["CarryOnKit"]
        ),
        .executableTarget(
            name: "DemoExample",
            dependencies: ["CarryOnKit"]
        ),
        .testTarget(
            name: "CarryOnKitTests",
            dependencies: ["CarryOnKit", "CCarryon"]
        ),
    ]
)
