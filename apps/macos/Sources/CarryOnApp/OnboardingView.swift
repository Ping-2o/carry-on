/*
 * OnboardingView.swift — first-run welcome, in the style of Apple's
 * "What's New" sheets: a hero glyph, three feature rows per page, one
 * primary action. Three pages (plan.md §1): the idea, what carries (L0–L4),
 * and the honest status of this build. Reduce-motion honoured.
 */

import SwiftUI

struct OnboardingView: View {
    var onFinish: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var step = 0

    private struct Feature: Identifiable {
        let id = UUID()
        let symbol: String
        let title: String
        let text: String
    }

    private struct Page {
        let symbol: String
        let title: String
        let subtitle: String
        let features: [Feature]
    }

    private let pages: [Page] = [
        Page(symbol: "laptopcomputer.and.arrow.down",
             title: "Welcome to Carry-On",
             subtitle: "Begin here — continue there.",
             features: [
                 Feature(symbol: "paperplane.fill", title: "Send to a nearby device",
                         text: "Pick a file, pick a device on your network, and it opens on the other side."),
                 Feature(symbol: "checkmark.seal.fill", title: "Verified end to end",
                         text: "Every transfer is hashed, sealed and checked. Tampered data is refused, never opened."),
                 Feature(symbol: "lock.shield.fill", title: "Private by design",
                         text: "Devices talk directly over TLS 1.3. No account, no cloud relay."),
             ]),
        Page(symbol: "square.stack.3d.up.fill",
             title: "What carries",
             subtitle: "Typed continuation — never process or memory migration (§3).",
             features: [
                 Feature(symbol: "doc.fill", title: "L0 · Files",
                         text: "The file itself, carried and verified."),
                 Feature(symbol: "arrow.up.forward.app.fill", title: "L1 · Open in an app",
                         text: "The receiving device launches the right app for it."),
                 Feature(symbol: "rectangle.stack.fill", title: "L2–L4 · Sessions and authority",
                         text: "Typed objects, structured sessions and single-writer hand-over — as adapters support them."),
             ]),
        Page(symbol: "flask.fill",
             title: "An honest preview",
             subtitle: "This build tells you exactly what has been proven.",
             features: [
                 Feature(symbol: "cpu", title: "Real engine, this device",
                         text: "Demo mode runs the real engine twice on this Mac over loopback TLS 1.3 — OBSERVED (LOCAL)."),
                 Feature(symbol: "qrcode", title: "Pair once",
                         text: "Scan a QR code to trust a device; later transfers need no pin."),
                 Feature(symbol: "exclamationmark.shield.fill", title: "Not yet a supported platform",
                         text: "Cross-device proof (§30) is still pending; the app never claims more than it observed."),
             ]),
    ]

    var body: some View {
        let page = pages[step]
        VStack(spacing: 0) {
            VStack(spacing: DS.Space.l) {
                GlyphTile(systemImage: page.symbol, size: 76)
                    .id(step)
                    .transition(reduceMotion ? .opacity : .scale(scale: 0.85).combined(with: .opacity))
                VStack(spacing: DS.Space.xs) {
                    Text(page.title)
                        .font(.system(size: 30, weight: .bold, design: .rounded))
                    Text(page.subtitle)
                        .font(.title3)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
            }
            .padding(.top, DS.Space.xxl + 8)

            VStack(alignment: .leading, spacing: DS.Space.l) {
                ForEach(page.features) { f in
                    HStack(alignment: .top, spacing: DS.Space.l) {
                        Image(systemName: f.symbol)
                            .font(.system(size: 26))
                            .foregroundStyle(DS.brand)
                            .frame(width: 40)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(f.title).font(.headline)
                            Text(f.text)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
            }
            .frame(maxWidth: 460, alignment: .leading)
            .padding(.top, DS.Space.xl)
            .id("features\(step)")
            .transition(.opacity)

            Spacer(minLength: DS.Space.l)

            HStack(spacing: DS.Space.s) {
                ForEach(pages.indices, id: \.self) { i in
                    Capsule()
                        .fill(i == step ? DS.brand : Color.secondary.opacity(0.3))
                        .frame(width: i == step ? 22 : 7, height: 7)
                        .onTapGesture { go(to: i) }
                        .accessibilityLabel("Page \(i + 1) of \(pages.count)")
                }
            }
            .padding(.bottom, DS.Space.l)

            HStack {
                Button("Skip") { onFinish() }
                    .keyboardShortcut(.escape)
                    .buttonStyle(.borderless)
                Spacer()
                if step > 0 {
                    Button("Back") { go(to: step - 1) }
                        .controlSize(.large)
                }
                Button(step < pages.count - 1 ? "Continue" : "Get Started") {
                    if step < pages.count - 1 { go(to: step + 1) } else { onFinish() }
                }
                .keyboardShortcut(.defaultAction)
                .controlSize(.large)
                .primaryActionStyle()
            }
            .padding(.horizontal, DS.Space.xl)
            .padding(.bottom, DS.Space.xl)
        }
        .frame(width: 620, height: 600)
        .background(.background)
    }

    private func go(to i: Int) {
        if reduceMotion {
            step = i
        } else {
            withAnimation(.smooth(duration: 0.35)) { step = i }
        }
    }
}
