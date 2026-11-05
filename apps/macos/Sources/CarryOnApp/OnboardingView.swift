/*
 * OnboardingView.swift — the 3-step continuity onboarding (plan.md §1):
 * 1. "Begin here → continue there" — typewriter ASCII logo.
 * 2. "What carries" — the L0–L4 ladder (exact text).
 * 3. "The lifecycle" — animated ASCII pipeline + honest notes.
 * Springs + matchedGeometryEffect on the step indicator; reduce-motion honored.
 */

import SwiftUI

struct OnboardingView: View {
    var onFinish: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Namespace private var stepGeometry
    @State private var step = 0

    private static let logo = """
    ┌──────────────────────────────────────────┐
    │  CARRY-ON                                │
    │  begin here → continue there             │
    └──────────────────────────────────────────┘
    """

    private static let ladder = """
    L4 ─ single-writer authority transfer
    L3 ─ structured session (document + unsaved + meta)
    L2 ─ typed objects + actions
    L1 ─ launcher activation
    L0 ─ files
    """

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                ForEach(0..<3, id: \.self) { i in
                    Capsule()
                        .fill(i == step ? Theme.accent : Color.secondary.opacity(0.3))
                        .frame(width: i == step ? 34 : 18, height: 5)
                        .matchedGeometryEffect(id: "step\(i)", in: stepGeometry)
                        .animation(carryonSpring(reduceMotion: reduceMotion), value: step)
                        .onTapGesture { withAnimation(carryonSpring(reduceMotion: reduceMotion)) { step = i } }
                }
            }
            .padding(.top, 28)

            Group {
                switch step {
                case 0: ideaStep
                case 1: ladderStep
                default: lifecycleStep
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            HStack {
                Button("Skip") { onFinish() }
                    .keyboardShortcut(.escape)
                Spacer()
                if step > 0 {
                    Button("Back") {
                        withAnimation(carryonSpring(reduceMotion: reduceMotion)) { step -= 1 }
                    }
                }
                Button(step < 2 ? "Continue" : "Open Carry-On") {
                    if step < 2 {
                        withAnimation(carryonSpring(reduceMotion: reduceMotion)) { step += 1 }
                    } else {
                        onFinish()
                    }
                }
                .keyboardShortcut(.return)
                .buttonStyle(.borderedProminent)
            }
            .padding(24)
        }
        .frame(minWidth: 640, minHeight: 440)
        .background(.background)
    }

    private var ideaStep: some View {
        VStack(spacing: 18) {
            AsciiBanner(text: Self.logo)
            Text("Begin here → continue there.")
                .font(.title2.weight(.semibold))
            Text("Carry-On is a cooperative continuation engine: start supported work on this Mac,\ncontinue the declared action on your other device. Typed continuation — never\nprocess or memory migration (spec §3).")
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Text("Demo mode runs the REAL engine on THIS device: two cores, loopback TLS 1.3.\nThat is OBSERVED (LOCAL) evidence — not physical cross-device transfer (spec §2/§30).")
                .font(Theme.monoSmall)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(40)
    }

    private var ladderStep: some View {
        VStack(spacing: 18) {
            Text("What carries").font(.title2.weight(.semibold))
            Text(Self.ladder)
                .font(Theme.monoTitle)
                .foregroundStyle(Theme.accent)
                .textSelection(.enabled)
            Text("Every level is honest about what crosses the wire: files (L0), launcher\nactivation (L1), typed objects and actions (L2), a structured session (L3),\nand single-writer authority transfer (L4).")
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(40)
    }

    private var lifecycleStep: some View {
        VStack(spacing: 18) {
            Text("The lifecycle").font(.title2.weight(.semibold))
            LifecycleDiagram()
            Text("The demo drives this exact pipeline through the engine's C ABI —\nfrom `carryon_core_open` to a verified evidence bundle.")
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(40)
    }
}
