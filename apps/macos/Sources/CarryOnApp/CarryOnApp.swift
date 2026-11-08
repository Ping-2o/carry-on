/*
 * CarryOnApp.swift — the SwiftUI app entry. Handles the headless
 * `--smoke-exit` flag used by Scripts/make_app.sh to verify the assembled
 * bundle (runs the full demo pipeline, exits 0 iff evidence verifies).
 *
 * Claim-label law (§2): the app labels the demo OBSERVED (LOCAL); it never
 * claims a supported platform (§30 unrun) or distribution signing (PLAT-006).
 */

import CarryOnKit
import SwiftUI

@main
struct CarryOnApp: App {
    @State private var app = AppState()

    init() {
        if CommandLine.arguments.contains("--smoke-exit") {
            Self.runSmokeExit()
        }
        if SnapshotRenderer.isActive {
            // Deterministic fixtures for CI screenshots; never touches the network.
            UIFixtures.install()
            UserDefaults.standard.set(true, forKey: "carryon.onboarding.done")
        }
    }

    /// Headless verification: run the demo pipeline, print the console, and
    /// exit the process (0 iff evidence verified ok:true). Never shows UI.
    static func runSmokeExit() -> Never {
        Task.detached(priority: .userInitiated) {
            let rc = await DemoConsole.runHeadless(config: DemoConfig())
            exit(Int32(rc))
        }
        while true {
            RunLoop.main.run(mode: .default, before: .distantPast)
        }
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(app)
                .tint(DS.brand)
                .task {
                    if SnapshotRenderer.isActive { await SnapshotRenderer.run(app: app) }
                }
        }
        .defaultSize(width: 1180, height: 780)
        .windowToolbarStyle(.unified)

        Settings {
            SettingsView()
                .environment(app)
                .frame(width: 420, height: 200)
        }

        // Menu commands: ⌘D run demo, ⌘N new session.
        .commands {
            CommandGroup(after: .newItem) {
                Button("Run Demo") {
                    Task { await app.runDemo() }
                }
                .keyboardShortcut("d", modifiers: .command)
                .disabled(app.demo.running)

                Button("New Session") {
                    app.newSession()
                }
                .keyboardShortcut("n", modifiers: .command)
                .disabled(app.demo.running)
            }
            CommandMenu("Go") {
                Button("Home") { app.pane = .home }.keyboardShortcut("1", modifiers: .command)
                Button("Send") { app.pane = .send }.keyboardShortcut("2", modifiers: .command)
                Button("Devices") { app.pane = .devices }.keyboardShortcut("3", modifiers: .command)
                Button("Activity") { app.pane = .activity }.keyboardShortcut("4", modifiers: .command)
                Divider()
                Button("PhotoCraft") { app.pane = .photocraft }.keyboardShortcut("5", modifiers: .command)
                Divider()
                Button("Engine Demo") { app.pane = .demo }.keyboardShortcut("6", modifiers: .command)
                Button("Transfer Console") { app.pane = .carry }
                Button("Pairing") { app.pane = .pairing }
                Button("Topology") { app.pane = .topology }
                Button("Lifecycle") { app.pane = .sessions }
                Button("Evidence") { app.pane = .evidence }.keyboardShortcut("7", modifiers: .command)
            }
        }
    }
}

/// Settings scene: demo defaults. Secret handling is fixed by law, so the only
/// knobs are the demo defaults; the pin display is ALWAYS truncated.
struct SettingsView: View {
    @Environment(AppState.self) private var app

    var body: some View {
        @Bindable var demo = app.demo
        Form {
            Picker("Default scenario", selection: $demo.config.scenario) {
                ForEach(DemoScenario.allCases) { s in
                    Text(s.title).tag(s)
                }
            }
            Picker("Default chunk size (§18.6)", selection: $demo.config.chunkSize) {
                Text("engine default").tag(UInt64(0))
                Text("4 KiB").tag(UInt64(4096))
                Text("16 KiB").tag(UInt64(16384))
                Text("64 KiB").tag(UInt64(65536))
            }
            LabeledContent("Identity keys") {
                Text("SECRET (EVD-006/§22.3) — never logged, shown, or bundled; only the SHA-256 pin hex, truncated, is displayed")
                    .font(Theme.monoSmall)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(20)
    }
}
