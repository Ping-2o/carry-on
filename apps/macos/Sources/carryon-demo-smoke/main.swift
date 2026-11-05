/*
 * main.swift — headless verification executable (plan.md verification gate):
 * runs the FULL L3 editor demo pipeline (stages 1–8) through the real engine
 * C ABI, prints the repo-style console + DISCLOSURE footer, and exits 0 only
 * if the evidence bundle verifies {"ok":true,...}.
 *
 * LOCAL evidence only (§2/§30): one device, loopback TLS 1.3.
 */

import CarryOnKit
import Foundation

// Allow: carryon-demo-smoke [editor|graph] [--chunk N] [--budget] [--suspend]
var config = DemoConfig()
var args = Array(CommandLine.arguments.dropFirst())
while let a = args.first {
    args.removeFirst()
    switch a {
    case "editor": config.scenario = .editorL3
    case "graph": config.scenario = .graph
    case "--chunk":
        config.chunkSize = UInt64(args.first ?? "") ?? 4096
        if !args.isEmpty { args.removeFirst() }
    case "--budget": config.budgetRejection = true
    case "--suspend": config.suspendResume = true
    default: break
    }
}

Task.detached(priority: .userInitiated) {
    let rc = await DemoConsole.runHeadless(config: config)
    exit(Int32(rc)) // flush stdio and exit with the verify verdict
}
dispatchMain() // the detached task exits the process when the run settles
