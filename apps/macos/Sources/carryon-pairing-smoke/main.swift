/*
 * main.swift — headless sides of the stage-5 pairing flow (§18.4), driving
 * the REAL engine through the C ABI via the same CarryOnKit code the macOS
 * app's "Pairing" screen uses. Two roles:
 *
 *   carryon-pairing-smoke arm [--name NAME]
 *       Arm as the DESTINATION: durable Keychain identity (PLAT-002), bound
 *       listener, six-digit code, QR payload printed for the source. Then
 *       blocks in awaitPairing with AUTO-CONFIRM (the operator reads both
 *       verification strings and drives the source side), pins the source,
 *       prints the result, and exits 0 only on Paired.
 *
 *   carryon-pairing-smoke join --addr IPv4:port --code NNNNNN [--payload URI]
 *       Join as the SOURCE (payload = the QR path; without = the provisional
 *       manual fallback), auto-confirm, learn the destination's decision,
 *       pin it in Keychain-sealed trust, exit 0 only on Paired.
 *
 *   carryon-pairing-smoke revoke --pin 64hex
 *       §18.4.9: revoke a peer from the persisted trust store (generation+1,
 *       reconnect blocked). Exit 0 on success.
 *
 *   carryon-pairing-smoke list
 *       The persisted trust records (pins only — public fingerprints).
 *
 * Labels (§2 law): loopback join → OBSERVED (LOCAL); a LAN join to a real
 * second device (e.g. the 2026-11-08 Galaxy A04 run) → PHYSICAL. Neither is
 * a supported platform (§30 unrun). The transcript printed here carries
 * names, pins, codes, and the verification string ONLY — never private keys
 * (EVD-006/§22.3, §18.4.8).
 */

import CarryOnKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data(("FAIL: \(message)\n").utf8))
    exit(2)
}

func info(_ line: String) {
    print(line)
}

var args = Array(CommandLine.arguments.dropFirst())
guard let role = args.first else {
    fail("usage: carryon-pairing-smoke arm|join|revoke|list [options]")
}
args.removeFirst()

let identityName = "carryon-mac-smoke"

// The durable identity (PLAT-002): Keychain-held DERs, stable pin.
let identity: DeviceIdentity
do {
    identity = try SecureStore.loadOrCreateIdentity(name: identityName)
    info("identity_pin:  \(try identity.pinHex())  (durable — Keychain, PLAT-002)")
} catch {
    fail("identity: \(error)")
}

// The persisted trust store (Keychain-sealed).
let trust: TrustStore = SecureStore.loadTrust() ?? (try! TrustStore.fromJSON("{\"peers\":{}}"))

switch role {
case "arm":
    let name = flagValue("--name", args) ?? Host.current().localizedName ?? "carryon-mac"
    let armed: PairingArmed
    do {
        armed = try PairingFlow.armDestination(identity: identity, deviceName: name)
        info("addr:    \(armed.displayAddr)")
        info("code:    \(armed.code)")
        info("payload: \(armed.payloadURI)")
        info("verification_string: shown by BOTH sides after the source connects — confirm match")
    } catch {
        fail("arm: \(error)")
    }
    let semaphore = DispatchSemaphore(value: 0)
    Task.detached {
        let state = await PairingFlow.awaitPairing(armed: armed, identity: identity, confirm: { step in
            if case .awaitingConfirm(let n, let p, let vs) = step {
                info("peer:    \(n)  (\(String(p.prefix(16)))…)")
                info("verify:  \(vs)  ← compare with the SOURCE screen (auto-confirm in smoke)")
            }
            return true // smoke auto-confirms; the operator reads both strings
        }) { line in
            FileHandle.standardOutput.write(Data((line + "\n").utf8))
        }
        switch state {
        case .paired(let n, let p, _):
            do {
                try trust.pair(name: n, pinHex: p, utc: PairingFlow.utcNow())
                try SecureStore.saveTrust(json: trust.toJSON())
                info("paired:  \(n) (\(String(p.prefix(16)))…) — pinned, trust persisted (Keychain)")
                exit(0)
            } catch {
                fail("persist pairing: \(error)")
            }
        case .refused(let why):
            fail("refused: \(why)")
        case .cancelled:
            fail("cancelled")
        default:
            fail("unexpected state \(state)")
        }
    }
    semaphore.wait()

case "join":
    guard let addr = flagValue("--addr", args) else { fail("join needs --addr IPv4:port") }
    guard let code = flagValue("--code", args) else { fail("join needs --code NNNNNN") }
    let payload = flagValue("--payload", args)
    let semaphore = DispatchSemaphore(value: 0)
    Task.detached {
        let state = await PairingFlow.joinDestination(
            identity: identity, addr: addr, code: code, payloadURI: payload,
            confirm: { step in
                if case .awaitingConfirm(_, let p, let vs) = step {
                    info("dest_pin: \(String(p.prefix(16)))…")
                    info("verify:   \(vs)  ← compare with the DESTINATION screen (auto-confirm in smoke)")
                }
                return true
            }) { line in
            FileHandle.standardOutput.write(Data((line + "\n").utf8))
        }
        switch state {
        case .paired(_, let p, _):
            do {
                try trust.pair(name: "destination", pinHex: p, utc: PairingFlow.utcNow())
                try SecureStore.saveTrust(json: trust.toJSON())
                info("paired:  \(String(p.prefix(16)))… pinned, trust persisted (Keychain)")
                exit(0)
            } catch {
                fail("persist pairing: \(error)")
            }
        case .refused(let why):
            fail("refused: \(why)")
        case .cancelled:
            fail("cancelled")
        default:
            fail("unexpected state \(state)")
        }
    }
    semaphore.wait()

case "revoke":
    guard let pin = flagValue("--pin", args) else { fail("revoke needs --pin 64hex") }
    do {
        let ok = try trust.revoke(pinHex: pin)
        guard ok else { fail("no such pinned peer") }
        try SecureStore.saveTrust(json: trust.toJSON())
        info("revoked: \(String(pin.prefix(16)))… (trust generation +1; reconnect will be refused §18.4.9)")
        exit(0)
    } catch {
        fail("revoke: \(error)")
    }

case "list":
    for r in (try? trust.records()) ?? [] {
        info("\(r.device_name)  \(String(r.pin_hex.prefix(16)))…  gen \(r.trust_generation)\(r.revoked ? "  [REVOKED]" : "")")
    }
    exit(0)

default:
    fail("unknown role \(role)")
}

func flagValue(_ flag: String, _ args: [String]) -> String? {
    guard let idx = args.firstIndex(of: flag), args.count > idx + 1 else { return nil }
    return args[idx + 1]
}
