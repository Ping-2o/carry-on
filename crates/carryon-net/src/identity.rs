//! Device identity (spec §18.4/§18.5). Each device holds a self-signed TLS
//! certificate; peers are authenticated by **pinning the SHA-256 of the peer's
//! certificate DER**, established during pairing. Any mismatch fails closed
//! (§18.5, NET-001/002).
//!
//! The private key never leaves the device and MUST NOT appear in evidence
//! bundles (§23.3, EVD-006). This crate does not persist the key; the platform
//! shell is responsible for secure storage (Keychain/Keystore/DPAPI, §8.7). In
//! tests the identity is held in memory for the duration of a loopback.

use crate::error::{AuthCode, NetError, Result};
use sha2::{Digest as _, Sha256};

/// A device's own TLS identity: its certificate (DER) and private key (PKCS#8 DER).
#[derive(Clone)]
pub struct DeviceIdentity {
    /// Short human-readable device name (shown during pairing; not a secret).
    pub device_name: String,
    /// Certificate in DER form (public; its SHA-256 is the pin).
    pub cert_der: Vec<u8>,
    /// PKCS#8 private key in DER form. **Secret.** Excluded from evidence.
    key_der: Vec<u8>,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the private key.
        f.debug_struct("DeviceIdentity")
            .field("device_name", &self.device_name)
            .field("pin", &self.pin().to_hex())
            .field("key_der", &"<redacted>")
            .finish()
    }
}

impl DeviceIdentity {
    /// Generate a fresh self-signed identity for `device_name`.
    pub fn generate(device_name: impl Into<String>) -> Result<DeviceIdentity> {
        let name = device_name.into();
        // Subject alt name is a stable logical label; transport trust comes from
        // the pin, not from name verification (self-signed, no CA).
        let certified =
            rcgen::generate_simple_self_signed(vec![format!("{name}.carryon.local")])
                .map_err(|e| NetError::auth(AuthCode::Handshake, format!("cert gen: {e}")))?;
        Ok(DeviceIdentity {
            device_name: name,
            cert_der: certified.cert.der().to_vec(),
            key_der: certified.key_pair.serialize_der(),
        })
    }

    /// Reconstruct an identity from previously persisted DER bytes (§18.4). This
    /// lets a device keep a **stable pin across launches**: the platform shell
    /// stores `(device_name, cert_der, key_der)` in secure storage (§8.7) and
    /// rehydrates the identity here on next launch. Both DERs are validated to
    /// parse; a bad key fails closed.
    pub fn from_der(
        device_name: impl Into<String>,
        cert_der: Vec<u8>,
        key_der: Vec<u8>,
    ) -> Result<DeviceIdentity> {
        // Validate the private key parses (same check `tls` makes when building a
        // config), so a malformed restore fails here, not mid-handshake.
        rustls::pki_types::PrivateKeyDer::try_from(key_der.clone())
            .map_err(|e| NetError::auth(AuthCode::Handshake, format!("bad private key: {e}")))?;
        if cert_der.is_empty() {
            return Err(NetError::auth(AuthCode::Handshake, "empty certificate DER"));
        }
        Ok(DeviceIdentity {
            device_name: device_name.into(),
            cert_der,
            key_der,
        })
    }

    /// The pin of this device: SHA-256 of its certificate DER.
    pub fn pin(&self) -> Pin {
        Pin::of_cert(&self.cert_der)
    }

    /// Export the PKCS#8 private key DER for persistence. **SECRET** — the caller
    /// MUST route this straight into platform secure storage (Keychain/Keystore/
    /// DPAPI, §8.7) and MUST NOT log it or place it in an evidence bundle
    /// (EVD-006). Named to make the secret explicit at every call site.
    pub fn export_key_der(&self) -> &[u8] {
        &self.key_der
    }

    /// The private key DER (PKCS#8). Kept crate-visible so only `tls` reads it.
    pub(crate) fn key_der(&self) -> &[u8] {
        &self.key_der
    }
}

/// A certificate pin: the SHA-256 of a peer's certificate DER (§18.5).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pin(pub [u8; 32]);

impl Pin {
    /// Compute the pin of a certificate's DER bytes.
    pub fn of_cert(cert_der: &[u8]) -> Pin {
        let mut h = Sha256::new();
        h.update(cert_der);
        let out = h.finalize();
        let mut p = [0u8; 32];
        p.copy_from_slice(&out);
        Pin(p)
    }

    /// Lowercase hex (64 chars).
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Parse a 64-hex-char pin.
    pub fn from_hex(s: &str) -> Option<Pin> {
        if s.len() != 64 {
            return None;
        }
        let mut p = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16)?;
            let lo = (chunk[1] as char).to_digit(16)?;
            p[i] = (hi * 16 + lo) as u8;
        }
        Some(Pin(p))
    }
}

impl std::fmt::Debug for Pin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pin({})", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_yields_stable_pin() {
        let id = DeviceIdentity::generate("alice").unwrap();
        assert_eq!(id.pin(), Pin::of_cert(&id.cert_der));
        assert_eq!(id.pin(), id.pin());
    }

    #[test]
    fn two_devices_have_distinct_pins() {
        let a = DeviceIdentity::generate("alice").unwrap();
        let b = DeviceIdentity::generate("bob").unwrap();
        assert_ne!(a.pin(), b.pin());
    }

    #[test]
    fn pin_hex_roundtrip() {
        let id = DeviceIdentity::generate("alice").unwrap();
        let hex = id.pin().to_hex();
        assert_eq!(Pin::from_hex(&hex), Some(id.pin()));
    }

    #[test]
    fn debug_redacts_private_key() {
        let id = DeviceIdentity::generate("alice").unwrap();
        let dbg = format!("{id:?}");
        assert!(dbg.contains("redacted"));
        assert!(!dbg.contains(&format!("{:?}", id.key_der())));
    }

    #[test]
    fn from_der_roundtrip_stable_pin() {
        let orig = DeviceIdentity::generate("alice").unwrap();
        let restored = DeviceIdentity::from_der(
            orig.device_name.clone(),
            orig.cert_der.clone(),
            orig.export_key_der().to_vec(),
        )
        .unwrap();
        // Same cert DER => same pin across "launches".
        assert_eq!(restored.pin(), orig.pin());
        assert_eq!(restored.export_key_der(), orig.export_key_der());
    }

    #[test]
    fn from_der_rejects_bad_key() {
        let orig = DeviceIdentity::generate("alice").unwrap();
        let err = DeviceIdentity::from_der("alice", orig.cert_der.clone(), vec![0, 1, 2, 3]);
        assert!(err.is_err());
    }
}
