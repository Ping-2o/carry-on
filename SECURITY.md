# Security policy

Carry-On moves session state between devices over the network, so security reports
are very welcome.

## Supported versions

Carry-On is pre-1.0 (workspace `0.1.0`, C ABI 1.3, SDK 1.3.0). Only the latest
`main` receives fixes.

## Reporting a vulnerability

**Please do not open a public issue.** Report privately through GitHub:

1. Open the [Security tab](https://github.com/Ping-2o/carry-on/security) of this
   repository.
2. Click **Report a vulnerability**
   ([direct link](https://github.com/Ping-2o/carry-on/security/advisories/new)).

Please include the affected component, the commit or version, reproduction steps
and the impact you expect.

This is a single-maintainer project, so reports are handled on a best-effort
basis. The fix is coordinated in the private advisory before public disclosure,
and you are credited in the advisory unless you prefer otherwise.

## Scope

Reports in these areas are especially valuable:

- pairing (PIN / QR, the mutual verification string) and trust-store handling
  (Keychain / Keystore)
- the TLS transport and discovery, including parsing of the mDNS TXT contract
- state import: digest verification, two-phase publish, budget admission (anything
  that gets corrupt, partial or tampered data published as valid)
- authority transfer: anything that allows two concurrent writers
- the C ABI and JNI boundary (memory safety, panics crossing FFI)

Out of scope: attacks that require an already-compromised device, and bugs in the
third-party apps themselves (PhotoCraft, Kiwix), which should be reported upstream.
