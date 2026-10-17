//! Unit tests for `master_pin`, owner-approved 2026-10-17: the right key pin
//! is accepted, a wrong one refused, and a mistyped one reported clearly.

use master_pin::{KeyPin, PinError, PinnedKey};
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

/// A self-signed P-256 certificate for `iw4l-prod`, made for these tests.
const CERT: &[u8] = include_bytes!("../fixtures/master-pin.der");
/// Its pin as the players' docs compute it:
/// `openssl x509 -inform der -pubkey -noout | openssl pkey -pubin -outform der | sha256sum`.
const PIN: &str = "9d3dff45236fa283592a180b9dd0cf2a8103d5fd0587ab337bb9da26b2b0ba7a";

fn verify(pin: &str) -> Result<(), rustls::Error> {
    let verifier = PinnedKey::new(KeyPin::parse(pin).unwrap());
    let name = ServerName::try_from("iw4l-prod").unwrap();
    verifier
        .verify_server_cert(
            &CertificateDer::from(CERT),
            &[],
            &name,
            &[],
            UnixTime::now(),
        )
        .map(|_| ())
}

#[test]
fn the_pin_matches_what_openssl_prints() {
    let pin = KeyPin::of_certificate(&CertificateDer::from(CERT)).unwrap();
    assert_eq!(pin.to_string(), PIN);
}

#[test]
fn the_right_pin_is_accepted() {
    assert_eq!(verify(PIN), Ok(()));
}

#[test]
fn a_wrong_pin_is_refused() {
    let wrong = format!("{}0", &PIN[..63]);
    let error = verify(&wrong).unwrap_err().to_string();
    assert!(error.contains("master key pin mismatch"), "{error}");
    assert!(error.contains(PIN), "{error}");
}

#[test]
fn a_pin_pasted_from_sha256sum_or_with_colons_parses() {
    let want = KeyPin::parse(PIN).unwrap();
    assert_eq!(KeyPin::parse(&format!("{PIN}  -\n")).unwrap(), want);
    assert_eq!(KeyPin::parse(&PIN.to_uppercase()).unwrap(), want);
    let colons: Vec<&str> = (0..32).map(|i| &PIN[i * 2..i * 2 + 2]).collect();
    assert_eq!(KeyPin::parse(&colons.join(":")).unwrap(), want);
}

#[test]
fn a_mistyped_pin_is_reported() {
    for bad in ["", "abc", &PIN[..62], &format!("{}g", &PIN[..63])] {
        assert_eq!(KeyPin::parse(bad), Err(PinError::Malformed(bad.to_owned())));
    }
    let message = KeyPin::parse("abc").unwrap_err().to_string();
    assert_eq!(
        message,
        "key pin `abc` is not a SHA-256: expected 64 hex digits"
    );
}
