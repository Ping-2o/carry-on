/*
 * Snapshots.swift — `CarryOnApp --snapshots <dir>` renders every main pane
 * in light and dark appearance from deterministic fixtures and writes PNGs,
 * then exits. Used by .github/workflows/ui-screenshots.yml.
 *
 * The real SwiftUI scene runs (WindowGroup, unified toolbar, sidebar), so the
 * capture shows what users see — including Liquid Glass on macOS 26. Capture
 * goes through the window server first (includes vibrancy/materials), with
 * an AppKit cacheDisplay fallback composited on the window background.
 */

import AppKit
import SwiftUI

@MainActor
enum SnapshotRenderer {
    /// The output directory when launched with `--snapshots <dir>`.
    static let outputDir: URL? = {
        let args = CommandLine.arguments
        guard let i = args.firstIndex(of: "--snapshots") else { return nil }
        let path = i + 1 < args.count ? args[i + 1] : "snapshots"
        return URL(fileURLWithPath: path, isDirectory: true)
    }()

    static var isActive: Bool { outputDir != nil }

    static let panes: [AppState.Pane] = [.home, .send, .devices, .activity, .demo, .evidence]
    static let size = NSSize(width: 1280, height: 820)

    private static var started = false

    /// Called once from the root view's task when snapshot mode is on.
    static func run(app: AppState) async {
        guard let dir = outputDir, !started else { return }
        started = true
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        log("snapshots → \(dir.path)")

        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        await settle(1.5)

        // Populate the engine-backed developer panes with a real local run.
        await app.runDemo()

        guard let window = mainWindow() else {
            log("error: no main window")
            exit(2)
        }
        window.setContentSize(size)
        window.center()
        window.makeKeyAndOrderFront(nil)

        var written = 0
        for (appearance, suffix) in [(NSAppearance.Name.aqua, "light"), (.darkAqua, "dark")] {
            NSApp.appearance = NSAppearance(named: appearance)
            for pane in panes {
                app.pane = pane
                await settle(1.0)
                if write(window: window, to: dir.appendingPathComponent("\(slug(pane))-\(suffix).png")) {
                    written += 1
                }
            }
            written += renderOnboarding(dir: dir, appearance: appearance, suffix: suffix) ? 1 : 0
        }
        log("wrote \(written) screenshots")
        exit(written > 0 ? 0 : 3)
    }

    // MARK: - Helpers

    private static func slug(_ pane: AppState.Pane) -> String {
        switch pane {
        case .home: return "home"
        case .send: return "send"
        case .devices: return "devices"
        case .activity: return "activity"
        case .photocraft: return "photocraft"
        case .demo: return "engine-demo"
        case .carry: return "transfer-console"
        case .pairing: return "pairing"
        case .topology: return "topology"
        case .sessions: return "lifecycle"
        case .evidence: return "evidence"
        }
    }

    private static func mainWindow() -> NSWindow? {
        NSApp.windows.first { $0.isVisible && !($0 is NSPanel) && $0.contentView != nil }
            ?? NSApp.windows.first { !($0 is NSPanel) }
    }

    private static func settle(_ seconds: Double) async {
        try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    }

    private static func renderOnboarding(dir: URL, appearance: NSAppearance.Name, suffix: String) -> Bool {
        let hosting = NSHostingView(rootView: OnboardingView(onFinish: {}).tint(DS.brand))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 620, height: 600),
                              styleMask: [.titled, .fullSizeContentView],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.appearance = NSAppearance(named: appearance)
        window.contentView = hosting
        window.center()
        window.orderFrontRegardless()
        RunLoop.main.run(until: Date().addingTimeInterval(1.0))
        let ok = write(window: window, to: dir.appendingPathComponent("onboarding-\(suffix).png"))
        window.orderOut(nil)
        return ok
    }

    @discardableResult
    private static func write(window: NSWindow, to url: URL) -> Bool {
        window.displayIfNeeded()
        guard let image = windowServerImage(window) ?? cachedImage(window) else {
            log("error: could not capture \(url.lastPathComponent)")
            return false
        }
        let rep = NSBitmapImageRep(cgImage: image)
        guard let data = rep.representation(using: .png, properties: [:]) else { return false }
        do {
            try data.write(to: url)
            log("  \(url.lastPathComponent) \(image.width)×\(image.height)")
            return true
        } catch {
            log("error: \(error)")
            return false
        }
    }

    /// Window-server composite (includes materials and glass). Own windows
    /// need no Screen Recording permission.
    private static func windowServerImage(_ window: NSWindow) -> CGImage? {
        let id = CGWindowID(window.windowNumber)
        guard let image = CGWindowListCreateImage(.null, .optionIncludingWindow, id,
                                                  [.boundsIgnoreFraming, .bestResolution]),
              image.width > 64, image.height > 64, !isBlank(image)
        else { return nil }
        return image
    }

    /// AppKit fallback: render the theme frame and composite it over the
    /// window background (vibrancy may come out transparent offscreen).
    private static func cachedImage(_ window: NSWindow) -> CGImage? {
        guard let view = window.contentView?.superview ?? window.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds)
        else { return nil }
        view.cacheDisplay(in: view.bounds, to: rep)
        let size = NSSize(width: rep.pixelsWide, height: rep.pixelsHigh)
        let out = NSImage(size: size)
        out.lockFocus()
        var appearanceBackground = NSColor.windowBackgroundColor
        window.effectiveAppearance.performAsCurrentDrawingAppearance {
            appearanceBackground = NSColor.windowBackgroundColor.usingColorSpace(.sRGB) ?? .windowBackgroundColor
        }
        appearanceBackground.setFill()
        NSRect(origin: .zero, size: size).fill()
        rep.draw(in: NSRect(origin: .zero, size: size))
        out.unlockFocus()
        return out.cgImage(forProposedRect: nil, context: nil, hints: nil)
    }

    /// True when every sampled pixel is fully transparent or pure black.
    private static func isBlank(_ image: CGImage) -> Bool {
        let w = 32, h = 32
        var pixels = [UInt8](repeating: 0, count: w * h * 4)
        guard let ctx = CGContext(data: &pixels, width: w, height: h, bitsPerComponent: 8,
                                  bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return false }
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
        var lit = 0
        for i in stride(from: 0, to: pixels.count, by: 4) where pixels[i + 3] > 0 {
            if pixels[i] > 8 || pixels[i + 1] > 8 || pixels[i + 2] > 8 { lit += 1 }
        }
        return lit < 8
    }

    private static func log(_ s: String) {
        FileHandle.standardError.write(Data(("[snapshots] " + s + "\n").utf8))
    }
}
