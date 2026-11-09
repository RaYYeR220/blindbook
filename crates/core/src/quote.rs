//! Verification of signed 1Click quotes.
//!
//! The matcher takes its reference price from the 1Click API and accepts a
//! quote only if it carries a valid ed25519 signature from the 1Click quote
//! manager key. The signed message is `base58(sha256(stable_json(fields)))`
//! over a fixed subset of request and quote fields plus the timestamp, the
//! same construction as `verifyQuoteSignature` in the official TypeScript SDK.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::price::Price;
use crate::Amount;

/// Public key the 1Click service signs quotes with.
pub const ONE_CLICK_MANAGER_PUB_KEY: &str = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";

const REQUEST_FIELDS: &[&str] = &[
    "dry",
    "swapType",
    "slippageTolerance",
    "originAsset",
    "depositType",
    "destinationAsset",
    "amount",
    "refundTo",
    "refundType",
    "recipient",
    "recipientType",
    "deadline",
];
/// Request fields the SDK includes only when truthy.
const REQUEST_FIELDS_IF_TRUTHY: &[&str] = &[
    "quoteWaitingTimeMs",
    "referral",
    "virtualChainRecipient",
    "virtualChainRefundRecipient",
    "customRecipientMsg",
];
const QUOTE_FIELDS: &[&str] = &[
    "amountIn",
    "amountInFormatted",
    "amountInUsd",
    "minAmountIn",
    "amountOut",
    "amountOutFormatted",
    "amountOutUsd",
    "minAmountOut",
];
/// Extra quote fields signed on non-dry quotes, included only when truthy.
const QUOTE_FIELDS_LIVE_IF_TRUTHY: &[&str] = &[
    "depositAddress",
    "depositMemo",
    "deadline",
    "timeWhenInactive",
    "timeEstimate",
    "refundFee",
    "withdrawFee",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuoteError {
    #[error("quote response is missing `{0}`")]
    Missing(&'static str),
    #[error("malformed field `{0}`")]
    Malformed(&'static str),
    #[error("signature does not verify against the 1Click key")]
    BadSignature,
    #[error("quote is for {got_in} -> {got_out}, expected {want_in} -> {want_out}")]
    WrongAssets { got_in: String, got_out: String, want_in: String, want_out: String },
}

/// A quote that passed signature verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedQuote {
    pub origin_asset: String,
    pub destination_asset: String,
    pub amount_in: Amount,
    pub amount_out: Amount,
    pub timestamp: String,
    /// The signed message (base58 of the sha256 digest), for audit logs.
    pub digest_b58: String,
}

impl VerifiedQuote {
    /// Price of `base` in `quote` units implied by this quote, for a market
    /// with the given base and quote assets. Selling base gives the bid;
    /// buying base gives the ask.
    pub fn price_for(&self, base: &str, quote: &str) -> Result<Price, QuoteError> {
        if self.origin_asset == base && self.destination_asset == quote {
            Price::new(self.amount_out, self.amount_in).map_err(|_| QuoteError::Malformed("amountIn"))
        } else if self.origin_asset == quote && self.destination_asset == base {
            Price::new(self.amount_in, self.amount_out).map_err(|_| QuoteError::Malformed("amountOut"))
        } else {
            Err(QuoteError::WrongAssets {
                got_in: self.origin_asset.clone(),
                got_out: self.destination_asset.clone(),
                want_in: base.to_string(),
                want_out: quote.to_string(),
            })
        }
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// The exact message string 1Click signs for a quote response.
pub fn signed_message(resp: &Value) -> Result<String, QuoteError> {
    let req = resp.get("quoteRequest").ok_or(QuoteError::Missing("quoteRequest"))?;
    let quote = resp.get("quote").ok_or(QuoteError::Missing("quote"))?;
    let timestamp = resp.get("timestamp").ok_or(QuoteError::Missing("timestamp"))?;
    let dry = req.get("dry").and_then(Value::as_bool).unwrap_or(false);

    // serde_json's default Map is ordered by key, which reproduces
    // json-stable-stringify's sorted, compact output.
    let mut obj = Map::new();
    for k in REQUEST_FIELDS {
        if let Some(v) = req.get(*k) {
            if !v.is_null() {
                obj.insert((*k).to_string(), v.clone());
            }
        }
    }
    for k in REQUEST_FIELDS_IF_TRUTHY {
        if let Some(v) = req.get(*k) {
            if truthy(v) {
                obj.insert((*k).to_string(), v.clone());
            }
        }
    }
    for k in QUOTE_FIELDS {
        if let Some(v) = quote.get(*k) {
            if !v.is_null() {
                obj.insert((*k).to_string(), v.clone());
            }
        }
    }
    if !dry {
        for k in QUOTE_FIELDS_LIVE_IF_TRUTHY {
            if let Some(v) = quote.get(*k) {
                if truthy(v) {
                    obj.insert((*k).to_string(), v.clone());
                }
            }
        }
    }
    obj.insert("timestamp".into(), timestamp.clone());

    let json = serde_json::to_string(&Value::Object(obj)).map_err(|_| QuoteError::Malformed("quote"))?;
    Ok(bs58::encode(Sha256::digest(json.as_bytes())).into_string())
}

fn decode_key(s: &str) -> Result<VerifyingKey, QuoteError> {
    let raw = bs58::decode(s.strip_prefix("ed25519:").unwrap_or(s))
        .into_vec()
        .map_err(|_| QuoteError::Malformed("public key"))?;
    let bytes: [u8; 32] = raw.try_into().map_err(|_| QuoteError::Malformed("public key"))?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| QuoteError::Malformed("public key"))
}

/// Verify a 1Click quote response against `pub_key` (normally
/// [`ONE_CLICK_MANAGER_PUB_KEY`]).
pub fn verify_quote(resp: &Value, pub_key: &str) -> Result<VerifiedQuote, QuoteError> {
    let message = signed_message(resp)?;
    let sig_str = resp
        .get("signature")
        .and_then(Value::as_str)
        .ok_or(QuoteError::Missing("signature"))?;
    let sig_raw = bs58::decode(sig_str.strip_prefix("ed25519:").unwrap_or(sig_str))
        .into_vec()
        .map_err(|_| QuoteError::Malformed("signature"))?;
    let sig_bytes: [u8; 64] = sig_raw.try_into().map_err(|_| QuoteError::Malformed("signature"))?;
    let key = decode_key(pub_key)?;
    key.verify(message.as_bytes(), &Signature::from_bytes(&sig_bytes))
        .map_err(|_| QuoteError::BadSignature)?;

    let req = &resp["quoteRequest"];
    let quote = &resp["quote"];
    let amount = |v: &Value, name: &'static str| -> Result<Amount, QuoteError> {
        v.as_str()
            .ok_or(QuoteError::Malformed(name))?
            .parse::<Amount>()
            .map_err(|_| QuoteError::Malformed(name))
    };
    Ok(VerifiedQuote {
        origin_asset: req["originAsset"].as_str().ok_or(QuoteError::Malformed("originAsset"))?.to_string(),
        destination_asset: req["destinationAsset"]
            .as_str()
            .ok_or(QuoteError::Malformed("destinationAsset"))?
            .to_string(),
        amount_in: amount(&quote["amountIn"], "amountIn")?,
        amount_out: amount(&quote["amountOut"], "amountOut")?,
        timestamp: resp["timestamp"].as_str().unwrap_or_default().to_string(),
        digest_b58: message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WNEAR: &str = "nep141:wrap.near";
    const USDC: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn live_sell_quote_verifies() {
        let q = verify_quote(&fixture("quote_sell"), ONE_CLICK_MANAGER_PUB_KEY).unwrap();
        assert_eq!(q.origin_asset, WNEAR);
        assert_eq!(q.amount_in, 10_000_000_000_000_000_000_000_000);
    }

    #[test]
    fn live_buy_quote_verifies() {
        let q = verify_quote(&fixture("quote_buy"), ONE_CLICK_MANAGER_PUB_KEY).unwrap();
        assert_eq!(q.origin_asset, USDC);
    }

    #[test]
    fn tampered_amount_is_rejected() {
        let mut r = fixture("quote_sell");
        let out: u128 = r["quote"]["amountOut"].as_str().unwrap().parse().unwrap();
        r["quote"]["amountOut"] = Value::String((out + 1).to_string());
        assert_eq!(verify_quote(&r, ONE_CLICK_MANAGER_PUB_KEY), Err(QuoteError::BadSignature));
    }

    #[test]
    fn tampered_timestamp_is_rejected() {
        let mut r = fixture("quote_buy");
        r["timestamp"] = Value::String("2030-01-01T00:00:00.000Z".into());
        assert_eq!(verify_quote(&r, ONE_CLICK_MANAGER_PUB_KEY), Err(QuoteError::BadSignature));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let other = "ed25519:C3jXhkGhEx88Gj7XKtUziJKXEBMRaJ67bWFkxJikVxZ2";
        assert_eq!(verify_quote(&fixture("quote_sell"), other), Err(QuoteError::BadSignature));
    }

    #[test]
    fn verified_quotes_give_a_tight_reference_mid() {
        let bid = verify_quote(&fixture("quote_sell"), ONE_CLICK_MANAGER_PUB_KEY)
            .unwrap()
            .price_for(WNEAR, USDC)
            .unwrap();
        let ask = verify_quote(&fixture("quote_buy"), ONE_CLICK_MANAGER_PUB_KEY)
            .unwrap()
            .price_for(WNEAR, USDC)
            .unwrap();
        assert!(bid.cmp_ratio(&ask).is_le(), "bid must not exceed ask");
        let mid = crate::price::midpoint(bid, ask, 150).unwrap();
        let one_near = mid.quote_for(1_000_000_000_000_000_000_000_000).unwrap();
        // sanity: between the two sides
        assert!(one_near >= bid.quote_for(1_000_000_000_000_000_000_000_000).unwrap());
        assert!(one_near <= ask.quote_for(1_000_000_000_000_000_000_000_000).unwrap());
    }

    #[test]
    fn quote_for_another_market_is_rejected() {
        let q = verify_quote(&fixture("quote_sell"), ONE_CLICK_MANAGER_PUB_KEY).unwrap();
        assert!(matches!(q.price_for("nep141:eth.omft.near", USDC), Err(QuoteError::WrongAssets { .. })));
    }
}
