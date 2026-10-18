//! A player's key. Each game keeps an Ed25519 key pair in `player.key`
//! (its 32-byte seed, hex) beside `settings.cfg`, made on the first run and
//! never sent. The public key's SHA-256, in hex, is the player's
//! **fingerprint**: what a host keeps an inventory by and lists its admins
//! by (`IW4L_ADMIN_KEYS`).
//!
//! On joining a room, a player proves it holds its key by signing the
//! room's id and the member id the master gave it for this join
//! ([`ProofContext`]): fresh for every join, so a proof seen once proves
//! nothing anywhere else.
#![forbid(unsafe_code)]

use std::path::Path;

use ring::rand::SecureRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use sha2::{Digest, Sha256};

pub const PUBLIC_KEY_BYTES: usize = 32;
pub const SIGNATURE_BYTES: usize = 64;
/// The key file's name, beside `settings.cfg`.
pub const KEY_FILE: &str = "player.key";
/// How many hex digits of a fingerprint are shown where space is short.
pub const SHORT_HEX: usize = 16;
const PROOF_DOMAIN: &[u8] = b"iw4l player proof v1";

pub struct PlayerKey {
    seed: [u8; 32],
    pair: Ed25519KeyPair,
}

impl std::fmt::Debug for PlayerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the seed.
        write!(f, "PlayerKey({})", self.fingerprint())
    }
}

/// What a join's proof signs: the room and the member id the master gave
/// this player for this join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProofContext {
    pub room: [u8; 16],
    pub member: [u8; 16],
}

impl ProofContext {
    fn message(&self) -> Vec<u8> {
        let mut message = PROOF_DOMAIN.to_vec();
        message.extend_from_slice(&self.room);
        message.extend_from_slice(&self.member);
        message
    }
}

/// A join's proof: the public key and its signature over the context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerProof {
    pub public: [u8; PUBLIC_KEY_BYTES],
    pub signature: [u8; SIGNATURE_BYTES],
}

impl PlayerProof {
    /// The fingerprint of the key this proof is for, when it signs
    /// `context`; `None` for a proof made for another room or member, or
    /// by another key.
    pub fn verify(&self, context: &ProofContext) -> Option<String> {
        UnparsedPublicKey::new(&ED25519, &self.public)
            .verify(&context.message(), &self.signature)
            .ok()
            .map(|()| fingerprint_of(&self.public))
    }
}

impl PlayerKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).expect("any 32 bytes are a seed");
        Self { seed, pair }
    }

    /// A new random key.
    pub fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| "no system randomness for a player key".to_owned())?;
        Ok(Self::from_seed(seed))
    }

    /// The key in `path`, or a new one written there (owner-only on Unix).
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = Self::generate()?;
                key.write(path)?;
                Ok(key)
            }
            Err(error) => Err(format!("read {}: {error}", path.display())),
        }
    }

    /// Writes the key's seed to `path` (not over an existing file).
    pub fn write(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let text = format!(
            "# This game's player key: who it is to every host. Keep it private; copy it to keep your identity.\n{}\n",
            hex(&self.seed)
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options
            .open(path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// A key file's text: comment lines, then the seed in hex.
    pub fn parse(text: &str) -> Result<Self, String> {
        let line = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or("no key in the file")?;
        let bytes = unhex(line).ok_or("the key is not 64 hex digits")?;
        let seed: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "the key is not 64 hex digits")?;
        Ok(Self::from_seed(seed))
    }

    pub fn public(&self) -> [u8; PUBLIC_KEY_BYTES] {
        let mut public = [0u8; PUBLIC_KEY_BYTES];
        public.copy_from_slice(self.pair.public_key().as_ref());
        public
    }

    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.public())
    }

    /// This key's proof for a join.
    pub fn prove(&self, context: &ProofContext) -> PlayerProof {
        let mut signature = [0u8; SIGNATURE_BYTES];
        signature.copy_from_slice(self.pair.sign(&context.message()).as_ref());
        PlayerProof {
            public: self.public(),
            signature,
        }
    }
}

/// A public key's fingerprint: its SHA-256 in lowercase hex.
pub fn fingerprint_of(public: &[u8; PUBLIC_KEY_BYTES]) -> String {
    hex(&Sha256::digest(public))
}

/// A fingerprint as written by a person (any case, spaces or colons
/// between digits), or `None` when it is not 64 hex digits.
pub fn normalize_fingerprint(text: &str) -> Option<String> {
    let digits: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect::<String>()
        .to_ascii_lowercase();
    (digits.len() == 64 && digits.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digits)
}

/// A fingerprint's first `SHORT_HEX` digits.
pub fn short(fingerprint: &str) -> &str {
    fingerprint.get(..SHORT_HEX).unwrap_or(fingerprint)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(room: u8, member: u8) -> ProofContext {
        ProofContext {
            room: [room; 16],
            member: [member; 16],
        }
    }

    #[test]
    fn a_proof_verifies_as_its_key_s_fingerprint() {
        let key = PlayerKey::from_seed([7; 32]);
        let proof = key.prove(&context(1, 2));
        assert_eq!(proof.verify(&context(1, 2)), Some(key.fingerprint()));
        assert_eq!(key.fingerprint().len(), 64);
        assert_eq!(short(&key.fingerprint()).len(), SHORT_HEX);
    }

    #[test]
    fn a_proof_from_another_key_is_refused() {
        let key = PlayerKey::from_seed([7; 32]);
        let other = PlayerKey::from_seed([8; 32]);
        let mut proof = key.prove(&context(1, 2));
        // Claiming another's public key with one's own signature.
        proof.public = other.public();
        assert_eq!(proof.verify(&context(1, 2)), None);
        // A damaged signature.
        let mut damaged = key.prove(&context(1, 2));
        damaged.signature[0] ^= 1;
        assert_eq!(damaged.verify(&context(1, 2)), None);
    }

    #[test]
    fn a_replayed_proof_is_refused() {
        let key = PlayerKey::from_seed([7; 32]);
        let proof = key.prove(&context(1, 2));
        // The same proof from another member of the room, or in another room.
        assert_eq!(proof.verify(&context(1, 3)), None);
        assert_eq!(proof.verify(&context(9, 2)), None);
    }

    #[test]
    fn a_key_file_round_trips_and_is_made_once() {
        let dir = std::env::temp_dir().join(format!("player-key-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(KEY_FILE);
        let made = PlayerKey::load_or_create(&path).unwrap();
        let read = PlayerKey::load_or_create(&path).unwrap();
        assert_eq!(made.fingerprint(), read.fingerprint());
        assert!(made.write(&path).is_err(), "never over an existing key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(PlayerKey::parse("# nothing\n").is_err());
        assert!(PlayerKey::parse("abc").is_err());
    }

    #[test]
    fn fingerprints_are_read_as_people_write_them() {
        let key = PlayerKey::from_seed([3; 32]);
        let fp = key.fingerprint();
        let spaced: String = fp
            .to_ascii_uppercase()
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(normalize_fingerprint(&spaced), Some(fp.clone()));
        assert_eq!(normalize_fingerprint(short(&fp)), None);
        assert_eq!(normalize_fingerprint("zz"), None);
    }

    #[test]
    fn the_debug_form_shows_no_seed() {
        let key = PlayerKey::from_seed([0xab; 32]);
        assert!(!format!("{key:?}").contains("abababab"));
    }
}
