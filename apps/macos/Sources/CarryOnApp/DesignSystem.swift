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
                Image(systemName: systemImage)
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(tone.color)
                    .frame(width: 34, height: 34)
                    .background(tone.color.opacity(0.12), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                VStack(alignment: .leading, spacing: 0) {
                    Text(value)
                        .font(.title2.weight(.semibold).monospacedDigit())
                        .contentTransition(.numericText())
                    Text(title)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// Friendly empty state for cards (ContentUnavailableView-style, card-sized).
struct EmptyStateView<Actions: View>: View {
    let title: String
    let message: String
    let systemImage: String
    var showsProgress = false
    private let actions: Actions

    init(_ title: String, message: String, systemImage: String, showsProgress: Bool = false,
         @ViewBuilder actions: () -> Actions) {
        self.title = title
        self.message = message
        self.systemImage = systemImage
        self.showsProgress = showsProgress
        self.actions = actions()
    }

    var body: some View {
        VStack(spacing: DS.Space.s) {
            ZStack {
                Circle().fill(DS.brand.opacity(0.10)).frame(width: 56, height: 56)
                Image(systemName: systemImage)
                    .font(.system(size: 24, weight: .medium))
                    .foregroundStyle(DS.brand)
                    .symbolEffect(.pulse, options: .repeating, isActive: showsProgress)
            }
            Text(title).font(.headline)
            Text(message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            actions.padding(.top, DS.Space.xs)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, DS.Space.l)
    }
}

extension EmptyStateView where Actions == EmptyView {
    init(_ title: String, message: String, systemImage: String, showsProgress: Bool = false) {
        self.init(title, message: message, systemImage: systemImage, showsProgress: showsProgress,
                  actions: { EmptyView() })
    }
}

/// Callout for errors and important notes.
struct Callout: View {
    let title: String
    let message: String
    var tone: StatusTone = .danger
    var systemImage = "exclamationmark.triangle.fill"

    var body: some View {
        HStack(alignment: .top, spacing: DS.Space.m) {
            Image(systemName: systemImage)
                .foregroundStyle(tone.color)
                .font(.title3)
            VStack(alignment: .leading, spacing: DS.Space.xxs) {
                Text(title).font(.headline)
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(DS.Space.m)
        .background(tone.color.opacity(0.10), in: RoundedRectangle(cornerRadius: DS.Radius.m, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DS.Radius.m, style: .continuous)
                .strokeBorder(tone.color.opacity(0.25), lineWidth: 1)
        )
        .accessibilityElement(children: .combine)
    }
}

// MARK: - Step track

enum FlowStepState: Equatable {
    case pending, active, done, failed
}

struct FlowStep: Identifiable, Equatable {
    let id: Int
    let title: String
    let systemImage: String
    let state: FlowStepState
}

/// Horizontal progress track for multi-step flows (Send wizard).
struct StepTrack: View {
    let steps: [FlowStep]

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            ForEach(steps) { step in
                StepNode(step: step)
                if step.id != steps.last?.id {
                    Capsule()
                        .fill(step.state == .done ? DS.brand : Color.secondary.opacity(0.22))
                        .frame(height: 3)
                        .frame(maxWidth: .infinity)
                        .padding(.horizontal, DS.Space.s)
                        .padding(.top, 17)
                }
            }
        }
        .accessibilityElement(children: .contain)
    }
}

private struct StepNode: View {
    let step: FlowStep

    private var fill: Color {
        switch step.state {
        case .done: return DS.brand
        case .active: return DS.brand.opacity(0.16)
        case .failed: return .red
        case .pending: return Color.secondary.opacity(0.12)
        }
    }

    private var glyph: String {
        switch step.state {
        case .done: return "checkmark"
        case .failed: return "xmark"
        default: return step.systemImage
        }
    }

    private var glyphColor: Color {
        switch step.state {
        case .done, .failed: return .white
        case .active: return DS.brand
        case .pending: return .secondary
        }
    }

    var body: some View {
        VStack(spacing: DS.Space.xs + 2) {
            ZStack {
                Circle().fill(fill)
                if step.state == .active {
                    Circle().strokeBorder(DS.brand, lineWidth: 2)
                }
                Image(systemName: glyph)
                    .font(.system(size: 14, weight: .bold))
                    .foregroundStyle(glyphColor)
                    .contentTransition(.symbolEffect(.replace))
            }
            .frame(width: 36, height: 36)
            Text(step.title)
                .font(.caption.weight(step.state == .active ? .semibold : .regular))
                .foregroundStyle(step.state == .pending ? .secondary : .primary)
                .lineLimit(1)
                .fixedSize()
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(step.title): \(accessibilityState)")
    }

    private var accessibilityState: String {
        switch step.state {
        case .done: return "done"
        case .active: return "current step"
        case .failed: return "failed"
        case .pending: return "not started"
        }
    }
}

// MARK: - Rows

/// Device row used by Home, Send and Devices.
struct DeviceRow<Trailing: View>: View {
    let device: DeviceItem
    var selected = false
    private let trailing: Trailing

    init(device: DeviceItem, selected: Bool = false, @ViewBuilder trailing: () -> Trailing) {
        self.device = device
        self.selected = selected
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: DS.Space.m) {
            Image(systemName: device.symbol)
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(selected ? Color.white : DS.brand)
                .frame(width: 38, height: 38)
                .background(selected ? AnyShapeStyle(DS.brandGradient) : AnyShapeStyle(DS.brand.opacity(0.11)),
                            in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(device.name).font(.body.weight(.medium)).lineLimit(1)
                Text(device.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: DS.Space.s)
            StatusPill(text: device.status.title, systemImage: device.status.symbol, tone: device.status.tone)
            trailing
        }
        .padding(.vertical, DS.Space.xs)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

extension DeviceRow where Trailing == EmptyView {
    init(device: DeviceItem, selected: Bool = false) {
        self.init(device: device, selected: selected, trailing: { EmptyView() })
    }
}

/// Session/activity row.
struct SessionRow: View {
    let item: SessionItem

    var body: some View {
        HStack(spacing: DS.Space.m) {
            Image(systemName: item.symbol)
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(item.tone.color)
                .frame(width: 34, height: 34)
                .background(item.tone.color.opacity(0.12), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(item.title).font(.body.weight(.medium)).lineLimit(1)
                Text(item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: DS.Space.s)
            VStack(alignment: .trailing, spacing: 2) {
                // §16.2 state names are shown verbatim (claim-label law).
                StatusPill(text: item.state.rawValue, tone: item.tone)
                if let when = item.when {
                    Text(when, format: .relative(presentation: .named))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .padding(.vertical, DS.Space.xs)
        .accessibilityElement(children: .combine)
    }
}

/// Two-column key/value row for detail cards.
struct DetailRow: View {
    let label: String
    let value: String
    var monospaced = false

    var body: some View {
        LabeledContent(label) {
            Text(value)
                .font(monospaced ? .callout.monospaced() : .callout)
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .font(.callout)
        .foregroundStyle(.secondary)
    }
}

/// "Technical details" disclosure — keeps §-references and engine internals
/// available for engineers without crowding the primary flow.
struct TechnicalDetails<Content: View>: View {
    var title = "Technical details"
    @State private var expanded = false
    private let content: Content

    init(_ title: String = "Technical details", @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        Card(padding: DS.Space.m) {
            DisclosureGroup(isExpanded: $expanded) {
                VStack(alignment: .leading, spacing: DS.Space.m) {
                    content
                }
                .padding(.top, DS.Space.s)
            } label: {
                Label(title, systemImage: "wrench.and.screwdriver")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: - Liquid Glass compatibility

#if compiler(>=6.2)
extension View {
    /// Floating functional-layer surface: Liquid Glass on macOS 26+,
    /// a standard material elsewhere.
    @ViewBuilder
    func floatingSurface(cornerRadius: CGFloat = DS.Radius.xl) -> some View {
        if #available(macOS 26.0, *) {
            self.glassEffect(.regular, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
        } else {
            self.background(.regularMaterial, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous).strokeBorder(DS.hairline))
                .shadow(color: .black.opacity(0.12), radius: 12, y: 4)
        }
    }

    /// The primary call to action.
    @ViewBuilder
    func primaryActionStyle() -> some View {
        if #available(macOS 26.0, *) {
            self.buttonStyle(.glassProminent)
        } else {
            self.buttonStyle(.borderedProminent)
        }
    }

    /// Secondary actions next to a primary one.
    @ViewBuilder
    func secondaryActionStyle() -> some View {
        if #available(macOS 26.0, *) {
            self.buttonStyle(.glass)
        } else {
            self.buttonStyle(.bordered)
        }
    }

    /// Mirrors hero content under the floating sidebar (macOS 26+).
    @ViewBuilder
    func extendsUnderSidebar() -> some View {
        if #available(macOS 26.0, *) {
            self.backgroundExtensionEffect()
        } else {
            self
        }
    }
}
#else
extension View {
    func floatingSurface(cornerRadius: CGFloat = DS.Radius.xl) -> some View {
        self.background(.regularMaterial, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous).strokeBorder(DS.hairline))
            .shadow(color: .black.opacity(0.12), radius: 12, y: 4)
    }

    func primaryActionStyle() -> some View {
        self.buttonStyle(.borderedProminent)
    }

    func secondaryActionStyle() -> some View {
        self.buttonStyle(.bordered)
    }

    func extendsUnderSidebar() -> some View {
        self
    }
}
#endif

// MARK: - Formatting

enum CarryFormat {
    static func bytes(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: value), countStyle: .file)
    }

    static func milliseconds(_ ms: Double) -> String {
        if ms >= 1000 { return String(format: "%.2f s", ms / 1000) }
        return String(format: "%.0f ms", ms)
    }
}
