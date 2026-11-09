//! The verifier's versioned nonce.
//!
//! ```text
//! 5628f6c6 | 00 | salt (4) | nonce deadline, i64 ns LE (8) | random (15)
//! ```
//!
//! The magic is the first four bytes of `sha256("versioned_nonce")`. The
//! verifier requires the salt to be valid, the intent deadline to be no later
//! than the nonce deadline, and the nonce deadline not to have passed.

pub const MAGIC: [u8; 4] = [0x56, 0x28, 0xf6, 0xc6];
pub const VERSION: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versioned {
    pub salt: [u8; 4],
    pub deadline_ns: i64,
}

pub fn encode(salt: [u8; 4], deadline_ns: i64, random: [u8; 15]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..4].copy_from_slice(&MAGIC);
    out[4] = VERSION;
    out[5..9].copy_from_slice(&salt);
    out[9..17].copy_from_slice(&deadline_ns.to_le_bytes());
    out[17..].copy_from_slice(&random);
    out
}

/// `None` for a legacy (random) nonce or an unknown version.
pub fn decode(nonce: &[u8; 32]) -> Option<Versioned> {
    if nonce[..4] != MAGIC || nonce[4] != VERSION {
        return None;
    }
    Some(Versioned {
        salt: nonce[5..9].try_into().ok()?,
        deadline_ns: i64::from_le_bytes(nonce[9..17].try_into().ok()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{sha256, unb64};

    #[test]
    fn magic_is_the_hash_prefix() {
        assert_eq!(sha256(b"versioned_nonce")[..4], MAGIC);
    }

    #[test]
    fn round_trip() {
        let n = encode([1, 2, 3, 4], 1_794_225_600_000_000_000, [9u8; 15]);
        assert_eq!(
            decode(&n),
            Some(Versioned {
                salt: [1, 2, 3, 4],
                deadline_ns: 1_794_225_600_000_000_000
            })
        );
    }

    #[test]
    fn decodes_an_sdk_nonce() {
        // From crates/core/tests/fixtures/sdk_orders.json: salt 252812b3, deadline 2030-01-01.
        let raw: [u8; 32] = unb64("Vij2xgAlKBKzAADVNTPoRhoBAQEBAQEBAQEBAQEBAQE=")
            .unwrap()
            .try_into()
            .unwrap();
        let v = decode(&raw).unwrap();
        assert_eq!(v.salt, [0x25, 0x28, 0x12, 0xb3]);
        assert_eq!(v.deadline_ns, 1_893_456_000_000_000_000);
    }

    #[test]
    fn legacy_nonces_are_not_versioned() {
        assert_eq!(decode(&[7u8; 32]), None);
        let mut n = encode([0; 4], 1, [0; 15]);
        n[4] = 1;
        assert_eq!(decode(&n), None);
    }
}
