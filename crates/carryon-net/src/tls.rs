//! TLS 1.3 configuration with certificate pinning (spec §8.6/§8.7/§18.5).
//!
//! Both peers present self-signed device certificates and authenticate each other
//! by **pinning the SHA-256 of the presented certificate** against the trust
//! store (§18.5). This is mutual TLS: the client verifies the server's pin and
//! the server verifies the client's pin. Real cryptographic signature checking is
//! delegated to the installed rustls crypto provider (`ring`) — no custom crypto
//! (§8.7). An unknown or revoked pin fails the handshake closed (NET-001/002).
//!
//! TLS 1.3 is the only protocol version enabled for state-changing transport, and
//! early data (0-RTT) is never used (§18.5, NET-002).

use crate::error::{AuthCode, NetError, Result};
use crate::identity::{DeviceIdentity, Pin};
use crate::trust::TrustStore;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::sync::Arc;

/// Install the process-wide default crypto provider (`ring`). Idempotent; safe to
/// call from every entry point. Returns the provider.
pub fn provider() -> Arc<CryptoProvider> {
    // `install_default` errors if already installed; that is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()))
}

/// A verifier that accepts exactly the peers the trust store pins (§18.5). Used
/// for both directions of mutual TLS. The accepted pin is captured so the
/// transport can report which peer actually connected.
#[derive(Debug)]
struct PinVerifier {
    trust: Arc<TrustStore>,
    provider: Arc<CryptoProvider>,
    accepted_pin: std::sync::Mutex<Option<Pin>>,
}

impl PinVerifier {
    fn new(trust: Arc<TrustStore>, provider: Arc<CryptoProvider>) -> Arc<PinVerifier> {
        Arc::new(PinVerifier {
            trust,
            provider,
            accepted_pin: std::sync::Mutex::new(None),
        })
    }

    /// Enforce the pin against the trust store, failing closed on unknown/revoked.
    fn check_pin(&self, end_entity: &CertificateDer<'_>) -> std::result::Result<(), rustls::Error> {
        let pin = Pin::of_cert(end_entity.as_ref());
        if !self.trust.is_trusted(&pin) {
            // Distinguish unknown vs revoked for the audit log, but both reject.
            return Err(rustls::Error::General(format!(
                "AUTH_PinMismatch: peer pin {} not trusted",
                pin.to_hex()
            )));
        }
        *self.accepted_pin.lock().unwrap() = Some(pin);
        Ok(())
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        self.check_pin(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl ClientCertVerifier for PinVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        self.check_pin(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Our own identity as rustls cert+key, cloned per config.
fn own_cert_and_key(
    id: &DeviceIdentity,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert = CertificateDer::from(id.cert_der.clone());
    let key = PrivateKeyDer::try_from(id.key_der().to_vec())
        .map_err(|e| NetError::auth(AuthCode::Handshake, format!("bad private key: {e}")))?;
    Ok((vec![cert], key))
}

/// Build a mutual-TLS **client** config pinned to the trust store (TLS 1.3 only).
/// Returns the config plus the verifier (to read back the accepted peer pin).
pub fn client_config(
    id: &DeviceIdentity,
    trust: Arc<TrustStore>,
) -> Result<(Arc<rustls::ClientConfig>, Arc<dyn PeerPin>)> {
    let provider = provider();
    let verifier = PinVerifier::new(trust, provider.clone());
    let (certs, key) = own_cert_and_key(id)?;
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::auth(AuthCode::Handshake, format!("tls13 only: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_client_auth_cert(certs, key)
        .map_err(|e| NetError::auth(AuthCode::Handshake, format!("client auth cert: {e}")))?;
    Ok((Arc::new(cfg), verifier))
}

/// Build a mutual-TLS **server** config pinned to the trust store (TLS 1.3 only).
pub fn server_config(
    id: &DeviceIdentity,
    trust: Arc<TrustStore>,
) -> Result<(Arc<rustls::ServerConfig>, Arc<dyn PeerPin>)> {
    let provider = provider();
    let verifier = PinVerifier::new(trust, provider.clone());
    let (certs, key) = own_cert_and_key(id)?;
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::auth(AuthCode::Handshake, format!("tls13 only: {e}")))?
        .with_client_cert_verifier(verifier.clone())
        .with_single_cert(certs, key)
        .map_err(|e| NetError::auth(AuthCode::Handshake, format!("server cert: {e}")))?;
    Ok((Arc::new(cfg), verifier))
}

/// Lets the transport read which peer pin a verifier accepted after the handshake.
pub trait PeerPin: Send + Sync + std::fmt::Debug {
    /// The pin accepted during the handshake, if the handshake completed.
    fn accepted_pin(&self) -> Option<Pin>;
}

impl PeerPin for PinVerifier {
    fn accepted_pin(&self) -> Option<Pin> {
        *self.accepted_pin.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_build_for_trusted_pair() {
        let server_id = DeviceIdentity::generate("server").unwrap();
        let client_id = DeviceIdentity::generate("client").unwrap();

        let mut server_trust = TrustStore::new();
        server_trust.pair("client", client_id.pin(), "t");
        let mut client_trust = TrustStore::new();
        client_trust.pair("server", server_id.pin(), "t");

        assert!(server_config(&server_id, Arc::new(server_trust)).is_ok());
        assert!(client_config(&client_id, Arc::new(client_trust)).is_ok());
    }
}
