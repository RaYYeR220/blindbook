//! Small encoding helpers.

use base64::Engine as _;
use sha2::{Digest, Sha256};

pub const NS_PER_S: u64 = 1_000_000_000;

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn unb64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

/// `ed25519:<base58>` for a 32-byte key or 64-byte signature.
pub fn ed25519_str(bytes: &[u8]) -> String {
    format!("ed25519:{}", bs58::encode(bytes).into_string())
}

/// Bytes of an `ed25519:<base58>` string.
pub fn ed25519_bytes(s: &str) -> Option<Vec<u8>> {
    bs58::decode(s.strip_prefix("ed25519:")?).into_vec().ok()
}

/// `YYYY-MM-DDTHH:MM:SS.000Z` for Unix seconds (civil-from-days, Howard Hinnant).
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// A NEAR account id as the protocol accepts it: 2 to 64 characters of
/// lowercase letters, digits and `-_.` separators, or a `0x` EVM address.
pub fn valid_account_id(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
                && !part.starts_with(['-', '_'])
                && !part.ends_with(['-', '_'])
        })
}

/// A 64-character lowercase hex implicit account.
pub fn is_implicit_hex(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use blindbook_core::intent::parse_rfc3339_ns;

    #[test]
    fn rfc3339_round_trips_through_the_core_parser() {
        for secs in [0u64, 1_794_225_600, 1_794_230_948, 4_102_444_800, 951_782_400] {
            assert_eq!(parse_rfc3339_ns(&rfc3339(secs)), Some(secs * NS_PER_S), "{secs}");
        }
        assert_eq!(rfc3339(1_794_225_600), "2026-11-09T12:00:00.000Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex(&[0, 1, 0xab, 0xff]), "0001abff");
        assert_eq!(unhex("0001abff"), Some(vec![0, 1, 0xab, 0xff]));
        assert_eq!(unhex("0g"), None);
        assert_eq!(unhex("abc"), None);
    }

    #[test]
    fn account_ids() {
        assert!(valid_account_id("alice.near"));
        assert!(valid_account_id("0x58da990a8f4a3a6ca7cb6315d68a140105917352"));
        assert!(valid_account_id("venue.blindbook-dev.testnet"));
        assert!(!valid_account_id("Alice.near"));
        assert!(!valid_account_id("a"));
        assert!(!valid_account_id("alice..near"));
        assert!(!valid_account_id("-alice.near"));
        assert!(is_implicit_hex(&"ab".repeat(32)));
        assert!(!is_implicit_hex(&"AB".repeat(32)));
        assert!(!is_implicit_hex("alice.near"));
    }

    #[test]
    fn ed25519_strings() {
        let s = ed25519_str(&[7u8; 32]);
        assert!(s.starts_with("ed25519:"));
        assert_eq!(ed25519_bytes(&s), Some(vec![7u8; 32]));
        assert_eq!(ed25519_bytes("secp256k1:abc"), None);
    }
}
