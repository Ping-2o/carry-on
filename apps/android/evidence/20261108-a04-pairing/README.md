# 2026-11-08 — Physical PIN/QR pairing mac ⇄ Galaxy A04 (stage 5, §18.4)

PHYSICAL cross-device evidence: two machines, two NICs, real LAN TLS 1.3,
§18.4 steps 1–9 end-to-end through the REAL engine (C ABI) on both sides.

## What ran

- **Destination**: CarryOn Android app on Galaxy A04 (SM-A045F), Pairing screen,
  "Show my QR (destination)" → "Accept pairing" (the provisional §18.4.3 channel).
  QR payload: `carryon://pair?v=1&n=SM-A045F&a=192.168.3.3:38507&k=1ab8a76465f85010…&r=899749`
  (the destination's durable Keystore identity pin `1ab8a76465f85010…`, addr, six-digit code).
- **Source**: `carryon-pairing-smoke join --addr 192.168.3.3:38507 --code 899749 --payload <URI>`
  on the Mac — the QR path: the destination is pinned from the payload BEFORE
  connecting (an already-authenticated channel; a MITM would fail the handshake).
- **User confirm on both sides (§18.4.5)**: the mac printed its verification
  string; the A04 displayed the same one in the VERIFY box; the human compared
  them and pressed Confirm on the phone.

## Results (see files)

- `mac-join-console.log` — source transcript: QR pin, verification string
  **f31006**, round-2 mutual confirm, `paired: 1ab8a76465f85010… pinned, trust
  persisted (Keychain)`.
- `a04-pairing-result.png` — destination screen: `paired: carryon-mac
  (48ec632e3b394977…) — trust persisted (sealed)`, the paired-devices list
  (carryon-mac, gen 0), console with the same `f31006`, and the disclosure footer.
- `mac-keychain-trust-list.log` — after the process exited, the mac's durable
  identity pin (Keychain, PLAT-002) and the persisted peer record.

## Honest labels (§2 law)

- Pairing flow (§18.4): **OBSERVED (PHYSICAL)** — real cross-device run above.
- PLAT-002: **OBSERVED on-device** — A04: AndroidKeyStore-wrapped identity +
  sealed trust (`device_{cert,key,trust}.bin`, ciphertext only, plaintext key
  DER never on disk — proven by the instrumented scan); mac: Keychain
  generic-password items, pin stable across process launches.
- Camera scan NOT implemented — manual payload/code entry is the honest
  fallback (typed into the mac smoke here; the A04 showed the QR for a real
  camera client).
- Revocation (§18.4.9): **OBSERVED (LOCAL)** — instrumented on-device tests
  (revoke → reconnect refused, generation+1) + mac smoke `revoke` command.
  No platform is "supported" (§30 unrun).

## Negative tests (on-device, instrumented — `PairingLoopbackInstrumentedTest`)

- wrong six-digit code echo → refused closed on BOTH sides;
- swapped cert → different verification string (§18.4.4 MITM model);
- revoked peer → pinned reconnect refused (client + server);
- destination decline → source NOT half-paired (round-2 mutual decision).
