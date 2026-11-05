/*
 * Theme.swift — visual tokens for the Carry-On macOS app. ASCII console style
 * (monospace, repo-style `key:  value`), adaptive Dark Mode colors, Dynamic
 * Type via system text styles.
 */

import CarryOnKit
import SwiftUI

enum Theme {
    /// The monospace face used by every engine/console surface.
    static let mono: Font = .system(size: 12, weight: .regular, design: .monospaced)
    static let monoSmall: Font = .system(size: 10.5, weight: .regular, design: .monospaced)
    static let monoTitle: Font = .system(size: 15, weight: .semibold, design: .monospaced)

    /// Stage / state colors (adaptive; also readable in Dark Mode).
    static let accent = Color.accentColor
    static let ok = Color.green
    static let skipped = Color.orange
    static let failed = Color.red
    static let pending = Color.secondary

    static let wire = Color.teal // TLS link color in the topology graph
}

/// Rendered console text for one `ConsoleLine` (repo-style `key:  value`).
extension ConsoleLine {
    /// Console rows render exactly like the smoke binary's stdout.
    var consoleText: String { rendered }
}

/// A spring used everywhere motion is allowed; reduce-motion paths skip it.
func carryonSpring(reduceMotion: Bool) -> Animation? {
    reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.8)
}
