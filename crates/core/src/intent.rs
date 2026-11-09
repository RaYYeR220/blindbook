//! Validation of pre-signed lot intents.
//!
//! A single invalid intent makes the verifier reject an entire
//! `execute_intents` batch, so the matcher checks every lot before it can be
//! crossed: the signature, the signer, the verifying contract, the deadline
//! and the exact shape of the `token_diff`.
//!
//! Supported signing standards:
//! - `nep413`: NEAR wallets (ed25519 over the NEP-413 hash),
//! - `erc191`: EVM wallets (`personal_sign`, secp256k1 recovery).

use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sha3::Keccak256;

use crate::engine::Side;
use crate::Amount;

/// NEP-413 prefix tag: 2^31 + 413.
const NEP413_TAG: u32 = 2_147_484_061;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntentError {
    #[error("unsupported signing standard `{0}`")]
    Standard(String),
    #[error("missing or malformed `{0}`")]
    Malformed(&'static str),
    #[error("signature does not verify")]
    BadSignature,
    #[error("signed by {got}, order owner is {want}")]
    WrongSigner { got: String, want: String },
    #[error("intent targets verifier {got}, expected {want}")]
    WrongVerifier { got: String, want: String },
    #[error("lot must carry exactly one token_diff intent")]
    NotASingleTokenDiff,
    #[error("token_diff does not match the lot: {0}")]
    WrongDiff(String),
    #[error("intent deadline {0} is invalid")]
    Deadline(String),
}

/// What a lot intent commits its signer to, after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedLot {
    pub signer_id: String,
    /// `ed25519:...` for NEP-413; `None` for erc191, where the key is the
    /// address itself.
    pub public_key: Option<String>,
    pub side: Side,
    pub limit_quote: Amount,
    pub deadline_ns: u64,
    /// Nonce as it appears in the signed payload (base64).
    pub nonce: String,
}

/// The expected lot: market assets, lot size and side.
pub struct LotSpec<'a> {
    pub verifier: &'a str,
    pub owner: &'a str,
    pub base: &'a str,
    pub quote: &'a str,
    pub lot: Amount,
    pub side: Side,
}

/// Validate one signed lot intent against `spec`.
pub fn check_lot(signed: &Value, spec: &LotSpec) -> Result<CheckedLot, IntentError> {
    let standard = signed
        .get("standard")
        .and_then(Value::as_str)
        .ok_or(IntentError::Malformed("standard"))?;
    let (inner, signer_check, public_key, nonce, verifier) = match standard {
        "nep413" => {
            let payload = signed.get("payload").ok_or(IntentError::Malformed("payload"))?;
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("payload.message"))?;
            let nonce_b64 = payload
                .get("nonce")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("payload.nonce"))?;
            let recipient = payload
                .get("recipient")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("payload.recipient"))?;
            let callback = payload.get("callbackUrl").and_then(Value::as_str);
            let pk = signed
                .get("public_key")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("public_key"))?;
            let sig = signed
                .get("signature")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("signature"))?;
            verify_nep413(message, nonce_b64, recipient, callback, pk, sig)?;
            let inner: Value = serde_json::from_str(message).map_err(|_| IntentError::Malformed("message json"))?;
            (inner, None, Some(pk.to_string()), nonce_b64.to_string(), recipient.to_string())
        }
        "erc191" => {
            let payload = signed
                .get("payload")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("payload"))?;
            let sig = signed
                .get("signature")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("signature"))?;
            let address = recover_erc191(payload, sig)?;
            let inner: Value = serde_json::from_str(payload).map_err(|_| IntentError::Malformed("payload json"))?;
            let nonce = inner
                .get("nonce")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("nonce"))?
                .to_string();
            let verifier = inner
                .get("verifying_contract")
                .and_then(Value::as_str)
                .ok_or(IntentError::Malformed("verifying_contract"))?
                .to_string();
            (inner, Some(address), None, nonce, verifier)
        }
        other => return Err(IntentError::Standard(other.to_string())),
    };

    if verifier != spec.verifier {
        return Err(IntentError::WrongVerifier { got: verifier, want: spec.verifier.to_string() });
    }
    let signer_id = inner
        .get("signer_id")
        .and_then(Value::as_str)
        .ok_or(IntentError::Malformed("signer_id"))?
        .to_string();
    if signer_id != spec.owner {
        return Err(IntentError::WrongSigner { got: signer_id, want: spec.owner.to_string() });
    }
    if let Some(address) = signer_check {
        if address != signer_id {
            return Err(IntentError::WrongSigner { got: address, want: signer_id });
        }
    }
    let deadline = inner
        .get("deadline")
        .and_then(Value::as_str)
        .ok_or(IntentError::Malformed("deadline"))?;
    let deadline_ns = parse_rfc3339_ns(deadline).ok_or_else(|| IntentError::Deadline(deadline.to_string()))?;

    let intents = inner
        .get("intents")
        .and_then(Value::as_array)
        .ok_or(IntentError::Malformed("intents"))?;
    if intents.len() != 1 || intents[0].get("intent").and_then(Value::as_str) != Some("token_diff") {
        return Err(IntentError::NotASingleTokenDiff);
    }
    let diff = intents[0]
        .get("diff")
        .and_then(Value::as_object)
        .ok_or(IntentError::Malformed("diff"))?;
    if diff.len() != 2 {
        return Err(IntentError::WrongDiff(format!("{} tokens in diff", diff.len())));
    }
    let amount = |token: &str| -> Result<i128, IntentError> {
        diff.get(token)
            .and_then(Value::as_str)
            .ok_or_else(|| IntentError::WrongDiff(format!("{token} missing")))?
            .parse::<i128>()
            .map_err(|_| IntentError::WrongDiff(format!("{token} not an integer")))
    };
    let base = amount(spec.base)?;
    let quote = amount(spec.quote)?;
    let lot = spec.lot as i128;
    let limit_quote = match spec.side {
        Side::Sell if base == -lot && quote > 0 => quote as Amount,
        Side::Buy if base == lot && quote < 0 => (-quote) as Amount,
        _ => {
            return Err(IntentError::WrongDiff(format!(
                "{:?} lot expects base {}{} but diff has base {base}, quote {quote}",
                spec.side,
                if spec.side == Side::Sell { "-" } else { "+" },
                spec.lot
            )))
        }
    };

    Ok(CheckedLot { signer_id, public_key, side: spec.side, limit_quote, deadline_ns, nonce })
}

/// The 32-byte hash a NEP-413 signature covers.
pub fn nep413_hash(message: &str, nonce: &[u8; 32], recipient: &str, callback_url: Option<&str>) -> [u8; 32] {
    let mut buf = Vec::with_capacity(64 + message.len() + recipient.len());
    buf.extend_from_slice(&NEP413_TAG.to_le_bytes());
    borsh_string(&mut buf, message);
    buf.extend_from_slice(nonce);
    borsh_string(&mut buf, recipient);
    match callback_url {
        Some(url) => {
            buf.push(1);
            borsh_string(&mut buf, url);
        }
        None => buf.push(0),
    }
    Sha256::digest(&buf).into()
}

fn borsh_string(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

fn verify_nep413(
    message: &str,
    nonce_b64: &str,
    recipient: &str,
    callback: Option<&str>,
    public_key: &str,
    signature: &str,
) -> Result<(), IntentError> {
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(nonce_b64)
        .map_err(|_| IntentError::Malformed("payload.nonce"))?
        .try_into()
        .map_err(|_| IntentError::Malformed("payload.nonce"))?;
    let pk: [u8; 32] = decode_prefixed(public_key, "ed25519:")
        .ok_or(IntentError::Malformed("public_key"))?
        .try_into()
        .map_err(|_| IntentError::Malformed("public_key"))?;
    let sig: [u8; 64] = decode_prefixed(signature, "ed25519:")
        .ok_or(IntentError::Malformed("signature"))?
        .try_into()
        .map_err(|_| IntentError::Malformed("signature"))?;
    let key = VerifyingKey::from_bytes(&pk).map_err(|_| IntentError::Malformed("public_key"))?;
    let hash = nep413_hash(message, &nonce, recipient, callback);
    key.verify(&hash, &Signature::from_bytes(&sig)).map_err(|_| IntentError::BadSignature)
}

/// Recover the lowercase `0x` address that `personal_sign`ed `payload`.
pub fn recover_erc191(payload: &str, signature: &str) -> Result<String, IntentError> {
    let raw = decode_prefixed(signature, "secp256k1:").ok_or(IntentError::Malformed("signature"))?;
    if raw.len() != 65 {
        return Err(IntentError::Malformed("signature"));
    }
    let v = raw[64];
    // The verifier accepts only a normalized recovery id; 27/28 would make the
    // whole batch fail, so such lots are refused up front.
    let recid = match v {
        0 | 1 => v,
        _ => return Err(IntentError::Malformed("signature recovery id must be 0 or 1")),
    };
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", payload.len(), payload);
    let digest = Keccak256::digest(prefixed.as_bytes());
    let sig = k256::ecdsa::Signature::from_slice(&raw[..64]).map_err(|_| IntentError::Malformed("signature"))?;
    let recid = k256::ecdsa::RecoveryId::from_byte(recid).ok_or(IntentError::Malformed("signature recovery id"))?;
    let key = k256::ecdsa::VerifyingKey::recover_from_prehash(&digest, &sig, recid)
        .map_err(|_| IntentError::BadSignature)?;
    let point = key.to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    Ok(format!("0x{}", hex_lower(&hash[12..])))
}

fn decode_prefixed(s: &str, prefix: &str) -> Option<Vec<u8>> {
    bs58::decode(s.strip_prefix(prefix).unwrap_or(s)).into_vec().ok()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fraction]Z` into Unix nanoseconds.
pub fn parse_rfc3339_ns(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, mo, da): (i64, u32, u32) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let (hms, frac) = match time.split_once('.') {
        Some((h, f)) => (h, f),
        None => (time, ""),
    };
    let mut t = hms.split(':');
    let (h, mi, se): (u64, u64, u64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut nanos: u64 = 0;
    if !frac.is_empty() {
        let digits: String = frac.chars().take(9).collect();
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        nanos = format!("{digits:0<9}").parse().ok()?;
    }
    // days from civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = (y2 - era * 400) as u64;
    let m = mo as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + da as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    if days < 0 {
        return None;
    }
    let secs = days as u64 * 86_400 + h * 3_600 + mi * 60 + se;
    Some(secs * 1_000_000_000 + nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    const BASE: &str = "nep141:wrap.near";
    const QUOTE: &str = "nep141:usdc.near";
    const LOT: Amount = 10_000_000_000_000_000_000_000_000;

    fn spec<'a>(owner: &'a str, side: Side) -> LotSpec<'a> {
        LotSpec { verifier: "intents.near", owner, base: BASE, quote: QUOTE, lot: LOT, side }
    }

    fn nep413_lot(key: &SigningKey, signer: &str, diff: Value, recipient: &str) -> Value {
        let message = json!({
            "deadline": "2026-11-10T12:00:00.000Z",
            "intents": [{ "intent": "token_diff", "diff": diff }],
            "signer_id": signer,
        })
        .to_string();
        let nonce = [3u8; 32];
        let hash = nep413_hash(&message, &nonce, recipient, None);
        let sig = key.sign(&hash);
        json!({
            "standard": "nep413",
            "payload": {
                "message": message,
                "nonce": base64::engine::general_purpose::STANDARD.encode(nonce),
                "recipient": recipient,
            },
            "public_key": format!("ed25519:{}", bs58::encode(key.verifying_key().to_bytes()).into_string()),
            "signature": format!("ed25519:{}", bs58::encode(sig.to_bytes()).into_string()),
        })
    }

    fn sell_diff(limit: &str) -> Value {
        json!({ BASE: format!("-{LOT}"), QUOTE: limit })
    }

    #[test]
    fn valid_nep413_sell_lot() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let lot = nep413_lot(&key, "alice.near", sell_diff("35400000"), "intents.near");
        let c = check_lot(&lot, &spec("alice.near", Side::Sell)).unwrap();
        assert_eq!(c.limit_quote, 35_400_000);
        assert_eq!(c.deadline_ns, parse_rfc3339_ns("2026-11-10T12:00:00Z").unwrap());
        assert!(c.public_key.unwrap().starts_with("ed25519:"));
    }

    #[test]
    fn valid_nep413_buy_lot() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let diff = json!({ BASE: LOT.to_string(), QUOTE: "-36100000" });
        let lot = nep413_lot(&key, "bob.near", diff, "intents.near");
        let c = check_lot(&lot, &spec("bob.near", Side::Buy)).unwrap();
        assert_eq!(c.limit_quote, 36_100_000);
    }

    #[test]
    fn tampered_message_fails_signature() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let mut lot = nep413_lot(&key, "alice.near", sell_diff("35400000"), "intents.near");
        let m = lot["payload"]["message"].as_str().unwrap().replace("35400000", "35400001");
        lot["payload"]["message"] = Value::String(m);
        assert_eq!(check_lot(&lot, &spec("alice.near", Side::Sell)), Err(IntentError::BadSignature));
    }

    #[test]
    fn signer_must_be_the_owner() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let lot = nep413_lot(&key, "mallory.near", sell_diff("35400000"), "intents.near");
        assert!(matches!(check_lot(&lot, &spec("alice.near", Side::Sell)), Err(IntentError::WrongSigner { .. })));
    }

    #[test]
    fn verifier_must_match() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        let lot = nep413_lot(&key, "alice.near", sell_diff("35400000"), "intents.testnet");
        assert!(matches!(check_lot(&lot, &spec("alice.near", Side::Sell)), Err(IntentError::WrongVerifier { .. })));
    }

    #[test]
    fn diff_must_be_exactly_one_lot_on_the_right_side() {
        let key = SigningKey::from_bytes(&[5u8; 32]);
        // a buy-shaped diff submitted as a sell lot
        let diff = json!({ BASE: LOT.to_string(), QUOTE: "-36100000" });
        let lot = nep413_lot(&key, "alice.near", diff, "intents.near");
        assert!(matches!(check_lot(&lot, &spec("alice.near", Side::Sell)), Err(IntentError::WrongDiff(_))));
        // half a lot
        let diff = json!({ BASE: format!("-{}", LOT / 2), QUOTE: "17000000" });
        let lot = nep413_lot(&key, "alice.near", diff, "intents.near");
        assert!(matches!(check_lot(&lot, &spec("alice.near", Side::Sell)), Err(IntentError::WrongDiff(_))));
        // a third token sneaks in
        let diff = json!({ BASE: format!("-{LOT}"), QUOTE: "35400000", "nep141:other.near": "1" });
        let lot = nep413_lot(&key, "alice.near", diff, "intents.near");
        assert!(matches!(check_lot(&lot, &spec("alice.near", Side::Sell)), Err(IntentError::WrongDiff(_))));
    }

    #[test]
    fn erc191_lot_recovers_the_address() {
        use k256::ecdsa::SigningKey as EcKey;
        let sk = EcKey::from_slice(&[9u8; 32]).unwrap();
        let point = sk.verifying_key().to_encoded_point(false);
        let addr = format!("0x{}", hex_lower(&Keccak256::digest(&point.as_bytes()[1..])[12..]));
        let payload = json!({
            "signer_id": addr,
            "verifying_contract": "intents.near",
            "deadline": "2026-11-10T12:00:00Z",
            "nonce": "AAAA",
            "intents": [{ "intent": "token_diff", "diff": sell_diff("35400000") }],
        })
        .to_string();
        let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", payload.len(), payload);
        let (sig, recid) = sk.sign_prehash_recoverable(&Keccak256::digest(prefixed.as_bytes())).unwrap();
        let mut raw = sig.to_bytes().to_vec();
        raw.push(recid.to_byte());
        let lot = json!({
            "standard": "erc191",
            "payload": payload,
            "signature": format!("secp256k1:{}", bs58::encode(raw).into_string()),
        });
        let c = check_lot(&lot, &spec(&addr, Side::Sell)).unwrap();
        assert_eq!(c.signer_id, addr);
        assert_eq!(c.limit_quote, 35_400_000);
        assert!(c.public_key.is_none());
    }

    #[test]
    fn unknown_standard_is_refused() {
        let lot = json!({ "standard": "raw_ed25519", "payload": "{}" });
        assert!(matches!(check_lot(&lot, &spec("a.near", Side::Sell)), Err(IntentError::Standard(_))));
    }

    #[test]
    fn rfc3339_parsing() {
        assert_eq!(parse_rfc3339_ns("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ns("2026-11-09T12:00:00.5Z"), Some(1_794_225_600_500_000_000));
        assert_eq!(parse_rfc3339_ns("2026-11-09T12:00:00.123456789Z"), Some(1_794_225_600_123_456_789));
        assert_eq!(parse_rfc3339_ns("2026-13-09T12:00:00Z"), None);
        assert_eq!(parse_rfc3339_ns("2026-11-09 12:00:00"), None);
    }
}
