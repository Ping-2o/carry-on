/*
 * carryon-photocraft-smoke — headless run of the SAME CarryOnKit PhotoCraft
 * send/receive path the app's pane uses, against two LIVE PhotoCraft desktop
 * apps on this machine.
 *
 * It exists so the product path (not only the Rust adapter) is exercised end to
 * end and can produce evidence without UI automation:
 *   sender  : inspect -> capture (split) -> cut -> serve budget windows
 *   receiver: descriptor -> §6.8 windows -> restore into the second live app
 *             -> engine oracle -> ack
 *
 * LABELS: OBSERVED (LOCAL) — two app processes + two cores on ONE machine,
 * loopback only. Not PHYSICAL, not §30.
 *
 * Usage:
 *   carryon-photocraft-smoke <config.json> [out.json]
 * config: {"source": {...live...}, "dest": {...live...},
 *          "data_dir": "...", "window_bytes": 262144}
 */

import CarryOnKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("PHOTOCRAFT_SMOKE_FAIL: \(message)\n".utf8))
    exit(1)
}

struct Side: Decodable {
    var addr: String
    var token_file: String
    var read_root: String
    var write_root: String
    var live: PhotoCraftLiveConfig {
        PhotoCraftLiveConfig(addr: addr, tokenFile: token_file,
                             readRoot: read_root, writeRoot: write_root)
    }
}

struct Config: Decodable {
    var source: Side
    var dest: Side
    var data_dir: String
    var window_bytes: UInt64?
}

/// Thread-safe event recorder for the sender (which runs off the main thread).
final class Recorder: @unchecked Sendable {
    private let lock = NSLock()
    private var _lines: [String] = []
    private var _listening: (addr: String, pin: String)?
    private var _finished: PhotoCraftResult?
    private var _error: String?
    let done = DispatchSemaphore(value: 0)

    var listening: (addr: String, pin: String)? {
        lock.lock(); defer { lock.unlock() }
        return _listening
    }
    var finished: PhotoCraftResult? {
        lock.lock(); defer { lock.unlock() }
        return _finished
    }
    var error: String? {
        lock.lock(); defer { lock.unlock() }
        return _error
    }
    var lines: [String] {
        lock.lock(); defer { lock.unlock() }
        return _lines
    }

    func apply(_ e: PhotoCraftEvent) {
        lock.lock()
        switch e {
        case .console(let l): _lines.append(l)
        case .listening(let addr, let pin): _listening = (addr, pin)
        case .finished(let r): _finished = r
        case .failed(let why): _error = why
        default: break
        }
        lock.unlock()
        if case .finished = e { done.signal() }
        if case .failed = e { done.signal() }
    }
}

// ---- main -------------------------------------------------------------------

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: carryon-photocraft-smoke <config.json> [out.json]") }
let cfgData = try Data(contentsOf: URL(fileURLWithPath: args[1]))
let cfg = try JSONDecoder().decode(Config.self, from: cfgData)
let outPath = args.count >= 3 ? args[2] : nil
let started = Date()

guard cfg.source.live.isValid, cfg.dest.live.isValid else {
    fail("a live PhotoCraft config is invalid (loopback addr, token file, roots)")
}

// Distinct per-run identities: each side pins the other by hand-visible pin.
let srcID = try DeviceIdentity.generate(name: "carryon-photocraft-src")
let dstID = try DeviceIdentity.generate(name: "carryon-photocraft-dst")
let srcPin = try srcID.pinHex()
let dstPin = try dstID.pinHex()

let senderConfig = PhotoCraftSendConfig(
    live: cfg.source.live, receiverPin: dstPin,
    dataDir: cfg.data_dir + "/src-core", windowBytes: cfg.window_bytes ?? 256 * 1024,
    encoding: "split", identity: srcID)
let recorder = Recorder()
let senderTask = Task.detached {
    do {
        _ = try PhotoCraftSender.run(senderConfig) { recorder.apply($0) }
    } catch {
        recorder.apply(.failed(error.localizedDescription))
    }
}

// Wait for the sender's listening address (bounded).
var listen: (addr: String, pin: String)?
for _ in 0..<1200 {
    if let l = recorder.listening { listen = l; break }
    if recorder.error != nil { break }
    usleep(50_000)
}
guard let listen else {
    fail("sender never reported a listening address: \(recorder.error ?? "timeout")")
}
print("sender listening at \(listen.addr) (pin \(listen.pin.prefix(12))…)")

// Give the sender a moment to enter its accept loop.
usleep(200_000)

let receiverConfig = PhotoCraftReceiveConfig(
    live: cfg.dest.live, sourceAddr: listen.addr, sourcePin: srcPin,
    dataDir: cfg.data_dir + "/dst-core", windowBytes: cfg.window_bytes ?? 256 * 1024,
    identity: dstID)
let receiverResult: PhotoCraftResult
do {
    receiverResult = try PhotoCraftReceiver.run(receiverConfig) { event in
        if case .console(let l) = event { print("receiver: \(l)") }
    }
} catch {
    fail("receiver: \(error)")
}

// The sender stops on the receiver's ack; wait briefly for its result.
_ = recorder.done.wait(timeout: .now() + 30)
let senderResult = recorder.finished
senderTask.cancel()

guard let restore = receiverResult.restore else { fail("receiver returned no restore report") }
guard restore.oracle.checked, restore.oracle.agreed else {
    fail("restore oracle disagreed: \(restore.oracle.detail)")
}
print("restored \(restore.restored.name) in the live app: tool=\(restore.restored.tool) zoom=\(restore.restored.zoom) layers=\(restore.restored.selectedLayers)")

let evidence: [String: Any] = [
    "experiment": "stage-10 PhotoCraft desktop UX via CarryOnKit (headless run of the app path)",
    "label": "OBSERVED (LOCAL)",
    "scope": "two live PhotoCraft apps + two Carry-On cores on ONE machine; loopback only",
    "channel": "PhotoCraft desktop JSON control channel (not MCP)",
    "encoding": "split",
    "window_bytes": cfg.window_bytes ?? 256 * 1024,
    "receiver_objects": receiverResult.objects,
    "receiver_windows": receiverResult.windows,
    "receiver_wire_bytes": receiverResult.wireBytes,
    "receiver_seconds": receiverResult.seconds,
    "sender_connections": senderResult?.windows ?? -1,
    "sender_wire_bytes": senderResult?.wireBytes ?? 0,
    "restored": [
        "name": restore.restored.name,
        "tool": restore.restored.tool,
        "zoom": restore.restored.zoom,
        "rotation": restore.restored.rotation,
        "selected_layers": restore.restored.selectedLayers,
        "dirty": restore.restored.dirty,
        "opened_path": restore.restored.openedPath,
    ],
    "oracle": [
        "checked": restore.oracle.checked,
        "agreed": restore.oracle.agreed,
        "detail": restore.oracle.detail,
    ],
    "workspace": restore.workspace,
    "total_seconds": Date().timeIntervalSince(started),
    "not_claimed": ["PHYSICAL (no second machine)", "platform supported (§30 unrun)"],
]
if let outPath {
    let data = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
    try data.write(to: URL(fileURLWithPath: outPath))
    print("evidence: \(outPath)")
}
print("PHOTOCRAFT_SMOKE_OK")
