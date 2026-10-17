//! Trust in a master by its key alone: the SHA-256 of the server
//! certificate's SubjectPublicKeyInfo, as
//! `openssl x509 -pubkey -noout | openssl pkey -pubin -outform der | sha256sum`
//! prints it. No CA file and no name: a master reached by bare IP, with a
//! self-signed certificate, is checked against the pin a player was given.
//! Reissuing the certificate with the same key keeps the pin.

use std::fmt;
use std::sync::Arc;

use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

/// SHA-256 of a public key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyPin([u8; 32]);

#[derive(Debug, PartialEq, Eq)]
pub enum PinError {
    /// Not 64 hex digits (colons allowed between bytes).
    Malformed(String),
}

impl fmt::Display for PinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(text) => write!(
                f,
                "key pin `{text}` is not a SHA-256: expected 64 hex digits"
            ),
        }
    }
}

impl std::error::Error for PinError {}

impl KeyPin {
    /// Parses a pin as `sha256sum` prints it (its trailing `  -` is ignored),
    /// in either case, with or without colons between bytes.
    pub fn parse(text: &str) -> Result<Self, PinError> {
        let malformed = || PinError::Malformed(text.trim().to_owned());
        let digits: Vec<u8> = text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .bytes()
            .filter(|&b| b != b':')
            .collect();
        if digits.len() != 64 {
            return Err(malformed());
        }
        let mut pin = [0u8; 32];
        for (byte, pair) in pin.iter_mut().zip(digits.chunks(2)) {
            let hex = |b: u8| (b as char).to_digit(16).ok_or_else(malformed);
            *byte = ((hex(pair[0])? << 4) | hex(pair[1])?) as u8;
        }
        Ok(Self(pin))
    }

    /// The pin of a certificate's public key.
    pub fn of_certificate(cert: &CertificateDer<'_>) -> Result<Self, rustls::Error> {
        let cert = webpki::EndEntityCert::try_from(cert)
            .map_err(|e| rustls::Error::General(format!("unreadable master certificate: {e}")))?;
        let digest = ring::digest::digest(
            &ring::digest::SHA256,
            cert.subject_public_key_info().as_ref(),
        );
        let mut pin = [0u8; 32];
        pin.copy_from_slice(digest.as_ref());
        Ok(Self(pin))
    }

    fn matches(&self, other: &Self) -> bool {
        // Every byte compared, so the time taken says nothing about the pin.
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl fmt::Display for KeyPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for KeyPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyPin({self})")
    }
}

/// Accepts exactly the master whose key has this pin. The handshake signature
/// is still checked against that key, so holding the certificate alone is not
/// enough; the name and dates are not, since the pin is the whole trust.
#[derive(Debug)]
pub struct PinnedKey {
    pin: KeyPin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedKey {
    pub fn new(pin: KeyPin) -> Self {
        Self {
            pin,
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinnedKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let pin = KeyPin::of_certificate(end_entity)?;
        if pin.matches(&self.pin) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "master key pin mismatch: expected {}, server has {pin}",
                self.pin
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A client configuration that trusts only the master with this pin.
pub fn client_config(pin: KeyPin) -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedKey::new(pin)))
        .with_no_client_auth()
}
