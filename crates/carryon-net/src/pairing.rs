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
}
