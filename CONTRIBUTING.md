# Contributing to Carry-On

Thanks for your interest! Carry-On is a small, evidence-driven project. This guide
explains how to propose a change and what "done" means here.

## Ground rule: labels are law

Every claim carries a label (`PROVED`, `IMPLEMENTED`, `OBSERVED`, `TARGET`,
`OPTIONAL`, `RESEARCH` or `UNSUPPORTED`) and points at its evidence (spec §2).

- A loopback or one-machine run is `OBSERVED (LOCAL)`. Cross-device evidence needs
  two physical devices and is labeled `OBSERVED (physical)`.
- A model check (Kani, stateright) is **model-level**, not an implementation proof.
- No platform is "supported" until the §30 acceptance harness passes on real
  hardware.
- If a PR changes a claim, update `docs/TARGETS.md`, `docs/FRONTIER.md` and/or
  `apps/README.md` in the same PR and add the evidence bundle under
  `docs/evidence/`.

A PR that upgrades a label without evidence will not be merged, however good the
code is.

## Workflow

1. For anything non-trivial, open an issue first (use the templates). Small fixes
   can go straight to a PR.
2. Branch from `main` with a prefix that matches the change: `feat/`, `fix/`,
   `docs/`, `chore/`, `research/`, `flagship/`, `apps/`, `sdk/`.
3. Keep PRs focused and fill in the PR template, including the claims table.
4. CI must be green before merge.

Commit subjects are short and imperative with an area prefix, for example
`core: refuse over-budget closures before transfer` or
`docs: sync FRONTIER with stage 9`.

## Local checks (the green gate)

These are exactly the commands CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets \
  --features carryon-ffi/test-panic,carryon-core/blocksum-checkpoint \
  --exclude carryon-adapter-kiwix -- -D warnings
cargo test --workspace \
  --features carryon-ffi/test-panic,carryon-core/blocksum-checkpoint \
  --exclude carryon-adapter-kiwix
```

The Kiwix adapter is excluded because its default `zim-backend` feature links a
locally built libzim (`docs/kiwix-bridge/BUILD-EVIDENCE.md`). CI checks its
pure-Rust lib instead:

```sh
cargo clippy -p carryon-adapter-kiwix --no-default-features --lib -- -D warnings
cargo test -p carryon-adapter-kiwix --no-default-features --lib
```

Supply chain (optional locally, runs in CI when dependencies change):

```sh
cargo install cargo-deny && cargo deny check
```

App and SDK builds are described under "Build and verify" in the README. CI builds
the macOS app with the demo smoke, the Android app with the `carryon-kit` AAR, and
the Swift SDK on every PR.

## Engine rules of thumb

- **The C ABI is append-only.** Never change or remove an existing symbol or struct
  layout; add new ones and bump the ABI minor. The SDKs follow the ABI
  (ABI 1.3 → SDK 1.3.0).
- **No custom crypto.** The transport is TLS 1.3 via rustls; identities and trust
  live in Keychain / Keystore.
- **Fail closed.** Unknown, corrupt or over-budget input is refused and never
  published as valid (CORE-004, §6.8).
- **No executable state.** Carry-On carries typed sessions, never code or process
  images.
- **No secrets in evidence.** Bearer tokens, keys and pairing codes must never
  appear in logs or evidence bundles (EVD-006).

## Security

Never open a public issue for a vulnerability; see [SECURITY.md](SECURITY.md).

## License of contributions

Unless you explicitly state otherwise, any contribution you intentionally submit
for inclusion in the work, as defined in the Apache-2.0 license, is dual-licensed
under MIT OR Apache-2.0, without any additional terms or conditions.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
