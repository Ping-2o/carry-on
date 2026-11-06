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
        }
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
