//! Device pairing (spec §18.4). Pairing establishes mutual trust by exchanging and
//! **pinning** each device's certificate SHA-256 over a user-mediated channel
//! (QR/short string). The pairing transcript excludes session content (§18.4.8)
//! and no private key is exchanged (only the public cert DER / its pin).
//!
//! # Headless model
//!
//! A real pairing uses proximity + a verification string shown on both devices
//! (§18.4.1-5). Here, [`PairingData`] is the exact payload that channel carries
//! (device name + certificate DER). [`pair_devices`] performs the mutual pin + the
//! verification-string match that both devices confirm, and returns each side's
//! trust store. This is the honest core of pairing minus the human UI, which is a
//! shell TARGET.

use crate::error::{AuthCode, NetError, Result};
use crate::identity::{DeviceIdentity, Pin};
use crate::trust::TrustStore;
use sha2::{Digest as _, Sha256};

/// The QR payload a **destination** displays (§18.4.1) and a source scans or
/// enters (§18.4.2). Wire format: a compact `carryon://pair` URI — small enough
/// for a version-1 QR, and typeable by hand only in its SHORT form (the six
/// digit code); the full pin never has to be typed (the stage-3 manual-IP UX
/// needed 64 hex — this replaces that as the primary path).
///
/// Fields (all data, §3.8): protocol version, device name, the destination's
/// listen address (`IPv4:port`), the destination's certificate pin (public
/// fingerprint; the source pins it BEFORE connecting so the first channel is
/// already authenticated), and a six-digit pairing code the source must echo
/// (fail closed on mismatch — the human part of §18.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingPayload {
    /// Payload protocol version. Only 1 exists; anything else fails closed.
    pub version: u32,
    pub device_name: String,
    /// The destination's listen address, `IPv4:port`.
    pub addr: String,
    /// The destination's certificate pin, 64 lowercase hex (public, §18.4.6).
    pub pin_hex: String,
    /// Six-digit pairing code (`000000`–`999999`) the source echoes.
    pub code: String,
}

impl PairingPayload {
    pub const SCHEME: &'static str = "carryon://pair";
    pub const VERSION: u32 = 1;

    /// Build a payload from its parts, validating every field (fail closed).
    pub fn new(
        device_name: impl Into<String>,
        addr: impl Into<String>,
        pin_hex: impl Into<String>,
        code: impl Into<String>,
    ) -> Result<PairingPayload> {
        let payload = PairingPayload {
            version: Self::VERSION,
            device_name: device_name.into(),
            addr: addr.into(),
            pin_hex: pin_hex.into(),
            code: code.into(),
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Serialize as the QR URI: `carryon://pair?v=1&n=…&a=…&k=…&r=…`.
    /// The device name is percent-encoded; the other fields are
    /// already-restricted alphabets.
    pub fn to_uri(&self) -> String {
        format!(
            "{}?v={}&n={}&a={}&k={}&r={}",
            Self::SCHEME,
            self.version,
            percent_encode(&self.device_name),
            self.addr,
            self.pin_hex,
            self.code,
        )
    }

    /// Parse a scanned/entered payload. Strict: unknown scheme, version,
    /// duplicates, out-of-range fields, or a malformed field fail closed
    /// (§3.8 — refuse, do not infer).
    pub fn parse(uri: &str) -> Result<PairingPayload> {
        let rest = uri.strip_prefix(Self::SCHEME).ok_or_else(|| {
            NetError::auth(
                AuthCode::Pairing,
                format!("not a carryon pairing payload (expected {SCHEME_PREFIX})",),
            )
        })?;
        let query = rest
            .strip_prefix('?')
            .ok_or_else(|| NetError::auth(AuthCode::Pairing, "missing query in pairing payload"))?;
        let mut version: Option<u32> = None;
        let mut name: Option<String> = None;
        let mut addr: Option<String> = None;
        let mut pin: Option<String> = None;
        let mut code: Option<String> = None;
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                NetError::auth(AuthCode::Pairing, "malformed pairing payload field")
            })?;
            let single = |slot: &mut Option<String>, what: &str| {
                if slot.is_some() {
                    Err(NetError::auth(
                        AuthCode::Pairing,
                        format!("duplicate {what} in pairing payload"),
                    ))
                } else {
                    *slot = Some(value.to_string());
                    Ok(())
                }
            };
            match key {
                "v" => {
                    if version.is_some() {
                        return Err(NetError::auth(
                            AuthCode::Pairing,
                            "duplicate v in pairing payload",
                        ));
                    }
                    version = Some(value.parse::<u32>().map_err(|_| {
                        NetError::auth(AuthCode::Pairing, "malformed v in pairing payload")
                    })?);
                }
                "n" => single(&mut name, "n")?,
                "a" => single(&mut addr, "a")?,
                "k" => single(&mut pin, "k")?,
                "r" => single(&mut code, "r")?,
                // Unknown keys are IGNORED (the §18.3 TXT minimalism rule,
                // applied to the pairing payload): never interpreted.
                _ => {}
            }
        }
        let payload = PairingPayload {
            version: version.ok_or_else(|| NetError::auth(AuthCode::Pairing, "missing v"))?,
            device_name: name
                .map(|n| percent_decode(&n))
                .ok_or_else(|| NetError::auth(AuthCode::Pairing, "missing n"))?,
            addr: addr.ok_or_else(|| NetError::auth(AuthCode::Pairing, "missing a"))?,
            pin_hex: pin.ok_or_else(|| NetError::auth(AuthCode::Pairing, "missing k"))?,
            code: code.ok_or_else(|| NetError::auth(AuthCode::Pairing, "missing r"))?,
        };
        payload.validate()?;
        Ok(payload)
    }

    fn validate(&self) -> Result<()> {
        if self.version != Self::VERSION {
            // Major-version mismatch fails closed (§18.2 applies to the
            // pairing payload too: a future format must not be half-parsed).
            return Err(NetError::auth(
                AuthCode::Pairing,
                format!("unsupported pairing payload version {}", self.version),
            ));
        }
        if self.device_name.is_empty() || self.device_name.len() > 64 {
            return Err(NetError::auth(
                AuthCode::Pairing,
                "device name must be 1..=64 chars",
            ));
        }
        if !is_ipv4_port(&self.addr) {
            return Err(NetError::auth(
                AuthCode::Pairing,
                "pairing address must be IPv4:port",
            ));
        }
        if Pin::from_hex(&self.pin_hex).is_none() {
            return Err(NetError::auth(
                AuthCode::Pairing,
                "pairing pin must be 64 hex",
            ));
        }
        if self.code.len() != 6 || !self.code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(NetError::auth(
                AuthCode::Pairing,
                "pairing code must be six digits",
            ));
        }
        Ok(())
    }
}

const SCHEME_PREFIX: &str = "carryon://pair";

/// Minimal IPv4:port check for the payload's address field (the engine does
/// not depend on app-level validators). Loopback is valid — it is how LOCAL
/// evidence is produced and honestly labeled.
pub fn is_ipv4_port(s: &str) -> bool {
    let Some((host, port_str)) = s.rsplit_once(':') else {
        return false;
    };
    let Ok(port) = port_str.parse::<u16>() else {
        return false;
    };
    if port == 0 || port_str.len() > 5 || port_str.starts_with('0') {
        return false;
    }
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().map(|n| n <= 255).unwrap_or(false)
                && !(p.len() > 1 && p.starts_with('0'))
        })
}

/// Percent-encode everything outside the RFC 3986 unreserved set (QR-safe,
/// no `+`-in-path ambiguity). Names are user-facing labels; non-ASCII is fine.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The inverse of [`percent_encode`]. A `%` not followed by two hex digits is
/// kept literally (the field then fails payload validation — fail closed).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            if let Ok(v) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                16,
            ) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Derive the six-digit pairing code for a destination's payload. Public and
/// deterministic given the pin + a fresh random-ish seed the caller supplies
/// (a listener port or counter): the code is the HUMAN part of §18.4.2 — the
/// source echoes it and the destination refuses a mismatch (fail closed).
/// It is NOT an authentication factor by itself; the verification_string
/// (§18.4.4) remains the mutual confirmation.
pub fn pairing_code(pin: &Pin, seed: u64) -> String {
    let mut h = Sha256::new();
    h.update(pin.0);
    h.update(seed.to_le_bytes());
    let out = h.finalize();
    let n = u32::from_be_bytes([out[0], out[1], out[2], out[3]]) % 1_000_000;
    format!("{n:06}")
}

/// The payload exchanged over the user-mediated pairing channel (§18.4.6). Public
/// only: a device name and its certificate DER (whose hash is the pin). Never a key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PairingData {
    pub device_name: String,
    /// Certificate DER (public). Its SHA-256 becomes the peer's pin.
    pub cert_der: Vec<u8>,
}

impl PairingData {
    /// The public pairing data for a device (safe to show/transmit).
    pub fn of(id: &DeviceIdentity) -> PairingData {
        PairingData {
            device_name: id.device_name.clone(),
            cert_der: id.cert_der.clone(),
        }
    }

    fn pin(&self) -> Pin {
        Pin::of_cert(&self.cert_der)
    }
}

/// The short verification string both devices display and a user confirms matches
/// (§18.4.1/4). Derived from both pins so a man-in-the-middle swapping either cert
/// produces a different string. Six hex chars is enough for a human cross-check.
pub fn verification_string(a: &Pin, b: &Pin) -> String {
    // Order-independent so both devices compute the same string.
    let (lo, hi) = if a.to_hex() <= b.to_hex() {
        (a, b)
    } else {
        (b, a)
    };
    let mut h = Sha256::new();
    h.update(lo.0);
    h.update(hi.0);
    let out = h.finalize();
    out[..3].iter().map(|b| format!("{b:02x}")).collect()
}

/// Perform a mutual pairing between two devices given the pairing data each one
/// received over the user-mediated channel. Returns `(local_trust, remote_trust)`
/// — each side's store with the other pinned. Both sides must compute the same
/// verification string (the human confirmation step, §18.4.5); a mismatch fails
/// closed, modeling a swapped certificate / MITM.
pub fn pair_devices(
    local: &DeviceIdentity,
    remote: &DeviceIdentity,
    paired_utc: &str,
) -> Result<(TrustStore, TrustStore)> {
    let local_sees = PairingData::of(remote);
    let remote_sees = PairingData::of(local);

    // Both independently derive the verification string; they must agree (§18.4.4).
    let vs_local = verification_string(&local.pin(), &local_sees.pin());
    let vs_remote = verification_string(&remote.pin(), &remote_sees.pin());
    if vs_local != vs_remote {
        return Err(NetError::auth(
            AuthCode::Handshake,
            "pairing verification strings differ (possible MITM)",
        ));
    }

    let mut local_trust = TrustStore::new();
    local_trust.pair(&local_sees.device_name, local_sees.pin(), paired_utc);
    let mut remote_trust = TrustStore::new();
    remote_trust.pair(&remote_sees.device_name, remote_sees.pin(), paired_utc);
    Ok((local_trust, remote_trust))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- existing pairing tests ----

    #[test]
    fn pairing_pins_both_directions() {
        let a = DeviceIdentity::generate("a").unwrap();
        let b = DeviceIdentity::generate("b").unwrap();
        let (ta, tb) = pair_devices(&a, &b, "t").unwrap();
        assert!(ta.is_trusted(&b.pin()));
        assert!(tb.is_trusted(&a.pin()));
        // Neither trusts a third party.
        let c = DeviceIdentity::generate("c").unwrap();
        assert!(!ta.is_trusted(&c.pin()));
    }

    #[test]
    fn verification_string_is_order_independent() {
        let a = DeviceIdentity::generate("a").unwrap();
        let b = DeviceIdentity::generate("b").unwrap();
        assert_eq!(
            verification_string(&a.pin(), &b.pin()),
            verification_string(&b.pin(), &a.pin())
        );
    }

    #[test]
    fn different_certs_give_different_verification() {
        let a = DeviceIdentity::generate("a").unwrap();
        let b = DeviceIdentity::generate("b").unwrap();
        let c = DeviceIdentity::generate("c").unwrap();
        assert_ne!(
            verification_string(&a.pin(), &b.pin()),
            verification_string(&a.pin(), &c.pin())
        );
    }

    // ---- QR pairing payload (§18.4.1-2, stage 5) ----

    fn sample_payload() -> PairingPayload {
        PairingPayload::new(
            "Galaxy A04",
            "192.168.1.20:48961",
            "a".repeat(64).as_str(),
            "428913",
        )
        .unwrap()
    }

    #[test]
    fn payload_uri_roundtrip() {
        let p = sample_payload();
        let parsed = PairingPayload::parse(&p.to_uri()).unwrap();
        assert_eq!(p, parsed);
        assert!(p.to_uri().starts_with("carryon://pair?v=1&"));
    }

    #[test]
    fn payload_name_percent_encoding_roundtrip() {
        let p = PairingPayload::new(
            "MacBook Pro & friend #1",
            "10.0.0.2:8080",
            "a".repeat(64).as_str(),
            "000000",
        )
        .unwrap();
        let uri = p.to_uri();
        assert!(
            !uri.contains(' ') && !uri.contains("&n=MacBook%20Pro%20&"),
            "raw name leaked: {uri}"
        );
        assert_eq!(PairingPayload::parse(&uri).unwrap(), p);
    }

    #[test]
    fn payload_rejects_unknown_scheme_and_version() {
        let pin = "a".repeat(64);
        assert!(PairingPayload::parse(
            "https://pair?v=1&n=a&a=1.2.3.4:1&k=&r=000000"
                .replace("&k=", &format!("&k={pin}"))
                .as_str()
        )
        .is_err());
        assert!(PairingPayload::parse(
            format!("carryon://pair?v=2&n=a&a=1.2.3.4:1&k={pin}&r=000000").as_str()
        )
        .is_err());
    }

    #[test]
    fn payload_rejects_bad_fields() {
        let good = sample_payload();
        // pin not 64 hex
        let bad_pin = PairingPayload::new("n", "1.2.3.4:80", "abc", "123456");
        assert!(bad_pin.is_err());
        // address not IPv4:port
        assert!(PairingPayload::new("n", "host:80", &good.pin_hex, "123456").is_err());
        assert!(PairingPayload::new("n", "1.2.3.4:0", &good.pin_hex, "123456").is_err());
        assert!(PairingPayload::new("n", "1.2.3.4", &good.pin_hex, "123456").is_err());
        // code not six digits
        assert!(PairingPayload::new("n", "1.2.3.4:80", &good.pin_hex, "12345").is_err());
        assert!(PairingPayload::new("n", "1.2.3.4:80", &good.pin_hex, "1234567").is_err());
        // empty name
        assert!(PairingPayload::new("", "1.2.3.4:80", &good.pin_hex, "123456").is_err());
    }

    #[test]
    fn payload_parse_missing_field_fails_closed() {
        let p = sample_payload();
        let uri = p.to_uri();
        let no_k = uri.split("&k=").next().unwrap().to_string();
        assert!(PairingPayload::parse(&no_k).is_err());
        // duplicate field
        assert!(PairingPayload::parse(&format!("{uri}&r=111111")).is_err());
    }

    #[test]
    fn payload_ignores_unknown_keys() {
        let p = sample_payload();
        let uri = format!("{}&pm=full&zz=1", p.to_uri());
        assert_eq!(PairingPayload::parse(&uri).unwrap(), p);
    }

    #[test]
    fn pairing_code_is_six_digits_and_seed_dependent() {
        let a = DeviceIdentity::generate("a").unwrap();
        let c1 = pairing_code(&a.pin(), 1);
        let c2 = pairing_code(&a.pin(), 2);
        assert_eq!(c1.len(), 6);
        assert!(c1.bytes().all(|b| b.is_ascii_digit()));
        assert_ne!(
            c1, c2,
            "different seeds should give different codes (overwhelmingly)"
        );
    }

    #[test]
    fn ipv4_port_validator_matches_stage3_semantics() {
        assert!(is_ipv4_port("127.0.0.1:48961"));
        assert!(is_ipv4_port("192.168.1.20:1"));
        assert!(!is_ipv4_port("1.2.3.4:0"));
        assert!(!is_ipv4_port("1.2.3.4:99999"));
        assert!(!is_ipv4_port("1.2.3:80"));
        assert!(!is_ipv4_port("1.2.3.4.5:80"));
        assert!(!is_ipv4_port("host:80"));
        assert!(!is_ipv4_port("1.2.3.4:"));
        assert!(!is_ipv4_port("01.2.3.4:80"));
    }
}
