/*
 * DesignSystem.swift — Carry-On's production design system for macOS.
 *
 * Follows the Apple Human Interface Guidelines:
 *  - Liquid Glass (macOS 26+) is used only in the functional layer (floating
 *    action bars, toolbar-style controls) — never on content surfaces.
 *  - Content lives on grouped, quiet surfaces with concentric corner radii.
 *  - Semantic colors + system text styles, so Dark Mode, Increase Contrast and
 *    Dynamic Type keep working.
 * Every glass API is gated by compiler + availability; older SDKs and older
 * macOS releases fall back to standard materials and bordered buttons.
 */

import AppKit
import SwiftUI

// MARK: - Tokens

enum DS {
    enum Space {
        static let xxs: CGFloat = 2
        static let xs: CGFloat = 4
        static let s: CGFloat = 8
        static let m: CGFloat = 12
        static let l: CGFloat = 16
        static let xl: CGFloat = 24
        static let xxl: CGFloat = 32
    }

    enum Radius {
        static let s: CGFloat = 8
        static let m: CGFloat = 12
        static let l: CGFloat = 16
        static let xl: CGFloat = 22
    }

    /// Readable line length for content panes.
    static let contentMaxWidth: CGFloat = 980

    /// Brand tint — calm teal, shared with the Android brand palette.
    static let brand = Color(red: 0.13, green: 0.60, blue: 0.52)
    static let brandDeep = Color(red: 0.07, green: 0.38, blue: 0.55)
    static let brandGradient = LinearGradient(
        colors: [Color(red: 0.16, green: 0.68, blue: 0.58), Color(red: 0.08, green: 0.42, blue: 0.60)],
        startPoint: .topLeading, endPoint: .bottomTrailing)

    /// Raised content surface (cards) — adapts to light/dark and contrast.
    static let surface = Color(nsColor: .controlBackgroundColor)
    /// Hairline used around cards.
    static let hairline = Color.primary.opacity(0.08)
}

// MARK: - Semantic tone

enum StatusTone {
    case neutral, info, success, warning, danger

    var color: Color {
        switch self {
        case .neutral: return .secondary
        case .info: return DS.brand
        case .success: return .green
        case .warning: return .orange
        case .danger: return .red
        }
    }
}

// MARK: - Surfaces

/// A quiet, grouped content surface.
struct Card<Content: View>: View {
    private let padding: CGFloat
    private let content: Content

    init(padding: CGFloat = DS.Space.l, @ViewBuilder content: () -> Content) {
        self.padding = padding
        self.content = content()
    }

    var body: some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(DS.surface, in: RoundedRectangle(cornerRadius: DS.Radius.l, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DS.Radius.l, style: .continuous)
                    .strokeBorder(DS.hairline, lineWidth: 1)
            )
            .shadow(color: .black.opacity(0.04), radius: 2, y: 1)
    }
}

/// A titled card section with an optional trailing accessory.
struct CardSection<Accessory: View, Content: View>: View {
    let title: String
    var systemImage: String?
    private let accessory: Accessory
    private let content: Content

    init(_ title: String, systemImage: String? = nil,
         @ViewBuilder accessory: () -> Accessory,
         @ViewBuilder content: () -> Content) {
        self.title = title
        self.systemImage = systemImage
        self.accessory = accessory()
        self.content = content()
    }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: DS.Space.m) {
                HStack(spacing: DS.Space.s) {
                    if let systemImage {
                        Image(systemName: systemImage)
                            .foregroundStyle(DS.brand)
                            .imageScale(.medium)
                    }
                    Text(title).font(.headline)
                    Spacer(minLength: DS.Space.s)
                    accessory
                }
                content
            }
        }
    }
}

extension CardSection where Accessory == EmptyView {
    init(_ title: String, systemImage: String? = nil, @ViewBuilder content: () -> Content) {
        self.init(title, systemImage: systemImage, accessory: { EmptyView() }, content: content)
    }
}

// MARK: - Page chrome

/// Large page header: brand glyph tile, title, subtitle, trailing accessory.
struct PageHeader<Accessory: View>: View {
    let title: String
    let subtitle: String
    let systemImage: String
    private let accessory: Accessory

    init(_ title: String, subtitle: String, systemImage: String,
         @ViewBuilder accessory: () -> Accessory) {
        self.title = title
        self.subtitle = subtitle
        self.systemImage = systemImage
        self.accessory = accessory()
    }

    var body: some View {
        HStack(alignment: .center, spacing: DS.Space.l) {
            GlyphTile(systemImage: systemImage, size: 46)
            VStack(alignment: .leading, spacing: DS.Space.xxs) {
                Text(title)
                    .font(.largeTitle.weight(.bold))
                    .accessibilityAddTraits(.isHeader)
                Text(subtitle)
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: DS.Space.l)
            accessory
        }
    }
}

extension PageHeader where Accessory == EmptyView {
    init(_ title: String, subtitle: String, systemImage: String) {
        self.init(title, subtitle: subtitle, systemImage: systemImage, accessory: { EmptyView() })
    }
}

/// Rounded brand-gradient tile carrying an SF Symbol (app-icon language).
struct GlyphTile: View {
    let systemImage: String
    var size: CGFloat = 36
    var gradient: LinearGradient = DS.brandGradient

    var body: some View {
        Image(systemName: systemImage)
            .font(.system(size: size * 0.46, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(gradient, in: RoundedRectangle(cornerRadius: size * 0.27, style: .continuous))
            .shadow(color: DS.brandDeep.opacity(0.25), radius: 4, y: 2)
            .accessibilityHidden(true)
    }
}

/// Scrollable, centered page container used by every redesigned pane.
struct PageScroll<Content: View>: View {
    private let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: DS.Space.xl) {
                content
            }
            .padding(.horizontal, DS.Space.xxl)
            .padding(.vertical, DS.Space.xl)
            .frame(maxWidth: DS.contentMaxWidth, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

// MARK: - Status

struct StatusPill: View {
    let text: String
    var systemImage: String?
    var tone: StatusTone = .neutral

    var body: some View {
        HStack(spacing: DS.Space.xs) {
            if let systemImage {
                Image(systemName: systemImage).imageScale(.small)
            }
            Text(text).lineLimit(1)
        }
        .font(.caption.weight(.semibold))
        .foregroundStyle(tone.color)
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(tone.color.opacity(0.13), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

/// A compact metric tile with animated numeric transitions.
struct MetricTile: View {
    let title: String
    let value: String
    let systemImage: String
    var tone: StatusTone = .info

    var body: some View {
        Card(padding: DS.Space.m + 2) {
            HStack(spacing: DS.Space.m) {
                Image(systemName

[Truncated after 247 lines. To continue, call readFile again with offset=248.]