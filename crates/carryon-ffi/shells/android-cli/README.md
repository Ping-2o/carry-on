# Carry-On Android C shell

Two minimal **non-Rust** system shells that link the Carry-On container runtime
(`libcarryon_ffi.so`) through the C ABI in [`../../include/carryon.h`](../../include/carryon.h)
and run on a physical Android device:

- `carryon_shell.c` — the full **import → carry → relaunch** lifecycle.
- `carryon_authority.c` — an on-device **L4 single-writer authority transfer**
  (§21.2): two cores + real TLS over the device's loopback, source relinquishes and
  destination becomes owner, verified via `carryon_may_mutate` on both sides.
- `carryon_xdev.c` — a **real cross-device** handoff + authority transfer: mac
  (source) ──LAN TLS──> android device (destination). Two physical machines, two
  NICs, mutual-pinned TLS over the wifi LAN — NOT loopback. One binary, roles
  `pin` / `source` / `dest`; identity persisted under an id-dir for a stable pin.
  Driven by `xdev_handoff.sh`.
- `carryon_appcont.c` — **real external-application continuation**: a real file open
  on the mac (file adapter, L1 content-hash identity, §10.7) is carried to the device
  and reconstructed byte-for-byte, then opened in a **real installed Android app**
  (Samsung Gallery) via the documented `ACTION_VIEW` intent (§10.3 PLAT-003). The
  destination re-checks the content-hash before launch — file identity guaranteed.
  Driven by `appcont_handoff.sh` (generates a PNG by default; set `FILE=` to carry
  your own). Honest L1: no unsaved in-memory app state is claimed.

## What it proves — and what it does not

This shell is the honest *portable-program-container* demonstration (spec §3.8):
a plain C program passes **data** across the FFI boundary and calls the runtime. It
never executes a foreign binary.

- **Proves (PLAT-001 physical execution):** the engine runs on real device hardware
  — the device's own `aarch64` CPU and filesystem. It opens a SQLite metadata DB,
  seals a cut, runs a source-off action whose oracle agrees, exports an evidence
  bundle, and **re-verifies that bundle on the device**.
- **Does NOT prove:** that Android is a *supported platform*. There is no APK, no
  packaging, and no signing identity — so PLAT-006 (packaging/signing in evidence)
  is explicitly out of scope here. No platform is claimed "supported" on the basis
  of this shell alone (spec §2/§30).

## Run it

```sh
export NDK=/path/to/android-ndk-r27c           # NDK r27+
export WORK="/Volumes/.../compilation"          # heavy build dir on external storage
# one rooted/adb-connected device attached
./build_and_run.sh
```

The script cross-compiles the runtime for `aarch64-linux-android`, compiles
`carryon_shell.c` with the NDK clang, pushes both to `/data/local/tmp/carryon`, and
runs the shell. Expected tail:

```
  action result:  {"oracle":{"agreed":true,...},"output":{"cost":26,...}}
  bundle verify:  {"message":"section hashes match","ok":true}
  DISCLOSURE: ran on the device CPU + filesystem through the C ABI (PLAT-001 ...).
```

Exit code `0` means every C-ABI call returned `CARRYON_OK` and the device-produced
evidence bundle re-verified on the device.

## Verified run (reference)

Reproduced on a Samsung Galaxy A04 (`SM-A045F`, `arm64-v8a`, Android 13 / API 33),
NDK r27c, min-API 21 build:

- `libcarryon_ffi.so` — ELF aarch64 shared object (full stack: core + net + rusqlite
  bundled + rustls, cross-compiled).
- `carryon_shell` — ELF aarch64 PIE executable (`interpreter /system/bin/linker64`).
- Device-side artifacts created under `/data/local/tmp/carryon/data/`:
  `metadata.sqlite3`, `journals/`, `objects/`, `android-shell-evidence.json`.
- Action oracle agreed (`dijkstra=Some(26) bellman_ford=Some(26)`); bundle
  re-verified on device (`section hashes match`).
- L4 authority transfer: matched receipt set (dest accept + source relinquish, same
  content-hash), epoch advanced `0 → 1`, ownership moved source → destination,
  `may_mutate` flipped on both sides. Exit `0`.
- **Cross-device** (`xdev_handoff.sh`, `MAC_IP=192.168.3.201`): mac source
  (`darwin arm64`) handed a sealed cut to the device over the wifi LAN (~77 ms RTT)
  under mutual-pinned TLS 1.3, then relinquished authority. Durable end-state,
  inspected on both machines:
  - device (owner): epoch 0 `remote/read_only_replica` (closed) + epoch 1
    `local/single_writer` (open, not ambiguous); both receipts persisted.
  - mac (relinquished): epoch 0 `local/single_writer` (closed) + epoch 1
    `remote/read_only_replica` (open).

  Two physical machines, two NICs — genuine PLAT-001 cross-device evidence. Still no
  APK/signing (PLAT-006 out of scope).
- **Real external-app continuation** (`appcont_handoff.sh`): a 512×256 PNG
  (`sha256 edaec484…`) captured on the mac was carried over the LAN and reconstructed
  on the device; the device-side `sha256sum` equalled the source hash (byte-for-byte),
  and the carried image opened in **Samsung Gallery**
  (`topResumedActivity=com.sec.android.gallery3d/.app.GalleryActivity`), rendering the
  exact gradient+XOR pattern. Screenshot saved to `android-out/gallery-proof.png`.
  A real installed third-party app displayed state carried from another machine.
