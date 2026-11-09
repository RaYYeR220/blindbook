//! Sealed order envelopes.
//!
//! Orders are encrypted in the trader's browser (or agent) to the matcher's
//! X25519 key, which only exists inside the TEE. The ciphertext is padded to a
//! fixed bucket size so its length does not reveal how many lots an order has.
//!
//! Layout: `version (1) || ephemeral public key (32) || nonce (24) || aead`.
//! Key: `HKDF-SHA256(ikm = X25519(eph, matcher), salt = eph_pub || matcher_pub,
//! info = "blindbook/v1/order")`. The version byte and ephemeral key are bound
//! as associated data. The plaintext is `len (u32 LE) || body || zero padding`.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

pub const VERSION: u8 = 1;
const INFO: &[u8] = b"blindbook/v1/order";
const HEADER: usize = 1 + 32 + 24;
const TAG: usize = 16;

/// Padded plaintext sizes. An order is always sealed into the smallest bucket
/// that fits it.
pub const BUCKETS: &[usize] = &[2_048, 4_096, 8_192, 16_384, 32_768, 65_536];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("order body of {0} bytes exceeds the largest bucket")]
    TooLarge(usize),
    #[error("unsupported envelope version {0}")]
    Version(u8),
    #[error("envelope is truncated")]
    Truncated,
    #[error("envelope does not decrypt with this key")]
    Decrypt,
    #[error("padding is malformed")]
    Padding,
}

/// The matcher's keypair, derived deterministically from a 32-byte seed held
/// in the TEE.
pub struct MatcherKey {
    secret: StaticSecret,
    public: PublicKey,
}

impl MatcherKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        // Domain-separate the seed so the same secret can back other keys.
        let hk = Hkdf::<Sha256>::new(Some(b"blindbook/v1/matcher-x25519"), &seed);
        let mut sk = [0u8; 32];
        hk.expand(b"x25519", &mut sk).expect("32 bytes is a valid HKDF length");
        let secret = StaticSecret::from(sk);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }

    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
        if sealed.len() < HEADER + TAG {
            return Err(EnvelopeError::Truncated);
        }
        if sealed[0] != VERSION {
            return Err(EnvelopeError::Version(sealed[0]));
        }
        let eph: [u8; 32] = sealed[1..33].try_into().unwrap();
        let nonce: [u8; 24] = sealed[33..57].try_into().unwrap();
        let shared = self.secret.diffie_hellman(&PublicKey::from(eph));
        let key = derive_key(shared.as_bytes(), &eph, &self.public.to_bytes());
        let cipher = XChaCha20Poly1305::new(&key.into());
        let padded = cipher
            .decrypt(XNonce::from_slice(&nonce), Payload { msg: &sealed[HEADER..], aad: &sealed[..33] })
            .map_err(|_| EnvelopeError::Decrypt)?;
        unpad(&padded)
    }
}

fn derive_key(shared: &[u8; 32], eph_pub: &[u8; 32], matcher_pub: &[u8; 32]) -> [u8; 32] {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(eph_pub);
    salt[32..].copy_from_slice(matcher_pub);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut key = [0u8; 32];
    hk.expand(INFO, &mut key).expect("32 bytes is a valid HKDF length");
    key
}

fn pad(body: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    let need = body.len() + 4;
    let size = *BUCKETS.iter().find(|b| **b >= need).ok_or(EnvelopeError::TooLarge(body.len()))?;
    let mut out = vec![0u8; size];
    out[..4].copy_from_slice(&(body.len() as u32).to_le_bytes());
    out[4..need].copy_from_slice(body);
    Ok(out)
}

fn unpad(padded: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    if padded.len() < 4 || !BUCKETS.contains(&padded.len()) {
        return Err(EnvelopeError::Padding);
    }
    let len = u32::from_le_bytes(padded[..4].try_into().unwrap()) as usize;
    if 4 + len > padded.len() || padded[4 + len..].iter().any(|b| *b != 0) {
        return Err(EnvelopeError::Padding);
    }
    Ok(padded[4..4 + len].to_vec())
}

/// Seal `body` to the matcher key. The caller supplies the randomness
/// (an ephemeral secret and a nonce), which keeps this function deterministic
/// and portable; the TypeScript SDK implements the same construction.
pub fn seal(
    matcher_pub: [u8; 32],
    body: &[u8],
    ephemeral_secret: [u8; 32],
    nonce: [u8; 24],
) -> Result<Vec<u8>, EnvelopeError> {
    let eph = StaticSecret::from(ephemeral_secret);
    let eph_pub = PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&PublicKey::from(matcher_pub));
    let key = derive_key(shared.as_bytes(), &eph_pub, &matcher_pub);
    let mut header = Vec::with_capacity(HEADER);
    header.push(VERSION);
    header.extend_from_slice(&eph_pub);
    let cipher = XChaCha20Poly1305::new(&key.into());
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &pad(body)?, aad: &header })
        .map_err(|_| EnvelopeError::Decrypt)?;
    header.extend_from_slice(&nonce);
    header.extend_from_slice(&ct);
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> MatcherKey {
        MatcherKey::from_seed([7u8; 32])
    }

    #[test]
    fn round_trip() {
        let k = key();
        let sealed = seal(k.public_bytes(), b"{\"side\":\"sell\"}", [1u8; 32], [2u8; 24]).unwrap();
        assert_eq!(k.open(&sealed).unwrap(), b"{\"side\":\"sell\"}");
    }

    #[test]
    fn ciphertext_length_hides_body_size() {
        let k = key();
        let small = seal(k.public_bytes(), &[b'a'; 10], [1u8; 32], [2u8; 24]).unwrap();
        let larger = seal(k.public_bytes(), &[b'a'; 1_900], [1u8; 32], [2u8; 24]).unwrap();
        assert_eq!(small.len(), larger.len());
        assert_eq!(small.len(), HEADER + 2_048 + TAG);
    }

    #[test]
    fn grows_to_the_next_bucket() {
        let k = key();
        let sealed = seal(k.public_bytes(), &[b'a'; 3_000], [1u8; 32], [2u8; 24]).unwrap();
        assert_eq!(sealed.len(), HEADER + 4_096 + TAG);
    }

    #[test]
    fn other_key_cannot_open() {
        let sealed = seal(key().public_bytes(), b"secret", [1u8; 32], [2u8; 24]).unwrap();
        let other = MatcherKey::from_seed([8u8; 32]);
        assert_eq!(other.open(&sealed), Err(EnvelopeError::Decrypt));
    }

    #[test]
    fn flipped_byte_is_rejected() {
        let k = key();
        let mut sealed = seal(k.public_bytes(), b"secret", [1u8; 32], [2u8; 24]).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert_eq!(k.open(&sealed), Err(EnvelopeError::Decrypt));
        let mut header_flip = seal(k.public_bytes(), b"secret", [1u8; 32], [2u8; 24]).unwrap();
        header_flip[5] ^= 1;
        assert!(k.open(&header_flip).is_err());
    }

    #[test]
    fn oversized_body_is_refused() {
        assert_eq!(
            seal(key().public_bytes(), &vec![0u8; 70_000], [1u8; 32], [2u8; 24]),
            Err(EnvelopeError::TooLarge(70_000))
        );
    }

    #[test]
    fn seed_derivation_is_stable() {
        assert_eq!(MatcherKey::from_seed([7u8; 32]).public_bytes(), key().public_bytes());
        assert_ne!(MatcherKey::from_seed([9u8; 32]).public_bytes(), key().public_bytes());
    }

    #[test]
    fn wrong_version_is_rejected() {
        let k = key();
        let mut sealed = seal(k.public_bytes(), b"x", [1u8; 32], [2u8; 24]).unwrap();
        sealed[0] = 9;
        assert_eq!(k.open(&sealed), Err(EnvelopeError::Version(9)));
    }
}
