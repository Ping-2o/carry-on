/*
 * DemoConsole.swift — headless runner: executes the demo pipeline and prints
 * repo-style `key:  value` console lines. Shared by the `carryon-demo-smoke`
 * executable and the app bundle's `--smoke-exit` flag (Scripts/make_app.sh).
 *
 * Exit contract: 0 only if the evidence bundle verifies ok:true (§23.3);
 * 3 on failure. The DISCLOSURE footer is printed verbatim from plan.md.
 */

import Foundation

public enum DemoConsole {
    /// Print console lines as they arrive (plain stdout, monospace assumed).
    public static func eventHandler() -> @Sendable (DemoEvent) -> Void {
        let lock = NSLock()
        return { event in
            lock.lock()
            defer { lock.unlock() }
            switch event {
            case .console(let line):
                if line.key.isEmpty {
                    print(line.value)
                } else {
                    print(line.rendered)
                }
            case .counters(let sent, let recv):
                FileHandle.standardError.write(Data("  [bytes] sent=\(sent) recv=\(recv)\n".utf8))
            case .stageStarted(let stage):
                FileHandle.standardError.write(Data("  [stage] \(stage.label) started\n".utf8))
            case .stageFinished(_, let outcome):
                let tag: String
                switch outcome {
                case .ok: tag = "ok"
                case .skipped(let why): tag = "skipped — \(why)"
                case .failed(let why): tag = "FAILED — \(why)"
                }
                FileHandle.standardError.write(Data("  [stage] \(tag)\n".utf8))
            case .mutationFlipped(let side, let session, let may):
                FileHandle.standardError.write(Data(
                    "  [authority] \(side.rawValue) \(session.prefix(8))… may_mutate=\(may)\n".utf8))
            case .sessionState(let side, let session, let state):
                FileHandle.standardError.write(Data(
                    "  [session] \(side.rawValue) \(session.prefix(8))… state=\(state.rawValue)\n".utf8))
            case .finished:
                break // the console + footer already carry the outcome
            case .failed(let why):
                FileHandle.standardError.write(Data("FAIL: \(why)\n".utf8))
            }
        }
    }

    /// Run one pipeline synchronously (blocking the caller — for CLI/`--smoke-exit`).
    /// Returns the process exit code: 0 iff evidence verified, else 3.
    @discardableResult
    public static func runHeadless(config: DemoConfig = DemoConfig()) async -> Int {
        let pipeline = DemoPipeline()
        do {
            let result = try await pipeline.run(config, emit: eventHandler())
            if !result.ok {
                FileHandle.standardError.write(Data(
                    "SMOKE FAIL: evidence did not verify (\(result.verifyMessage))\n".utf8))
                return 3
            }
            return 0
        } catch {
            FileHandle.standardError.write(Data("SMOKE FAIL: \(error)\n".utf8))
            return 3
        }
    }
}
