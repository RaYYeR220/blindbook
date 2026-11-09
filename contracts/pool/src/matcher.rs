//! Matcher I/O (PROTOCOL §3): the public `input_data` the pool builds, and the
//! signed output it receives back through OutLayer's yield/resume.

use near_sdk::bs58;
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::AccountId;
use serde_json::value::RawValue;

use crate::types::{BookCounts, KeysReport, Market, MarketSummary, Mid, Params};

pub const OP_BATCH: &str = "batch";
pub const OP_KEYS: &str = "keys";

/// §3.1. Field order is the wire order.
#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Input<'a> {
    pub v: u32,
    pub op: &'a str,
    pub network: &'a str,
    pub verifier: &'a AccountId,
    pub venue_account: &'a AccountId,
    pub now_ns: String,
    pub markets: &'a [Market],
    pub params: &'a Params,
    pub orders: Vec<InputOrder>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct InputOrder {
    pub id: String,
    pub seq: u64,
    pub market: String,
    pub sealed: String,
}

/// Exact serialized length of one `InputOrder`, given the length of its
/// market id (validated to need no JSON escaping) and of its envelope.
pub fn input_order_len(id: u64, market_len: usize, sealed_len: usize) -> usize {
    // {"id":"<id>","seq":<id>,"market":"<m>","sealed":"<b64>"}
    40 + 2 * digits(id) + market_len + base64_len(sealed_len)
}

pub fn base64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

fn digits(mut n: u64) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// §3.2 outer object, or `{"error": "<code>"}` when the module could not sign (§3.4).
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Output {
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub sig: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct BatchResult<'a> {
    pub v: u32,
    pub op: String,
    /// Present only on a failed run (§3.4); nothing else in the result counts then.
    #[serde(default)]
    pub error: Option<String>,
    /// Hex sha256 of the input this result answers.
    #[serde(default)]
    pub input_sha256: String,
    #[serde(default)]
    pub ingested: Vec<String>,
    /// Orders neither ingested nor rejected: they stay pending.
    #[serde(default)]
    pub deferred: Vec<String>,
    #[serde(default)]
    pub rejected: Vec<RawRejection>,
    #[serde(default, borrow)]
    pub batches: Vec<RawSummary<'a>>,
    /// One array of signed verifier payloads per sub-batch, passed through byte for byte.
    #[serde(default, borrow)]
    pub settle: Vec<Vec<&'a RawValue>>,
    #[serde(default)]
    pub book: Option<BookCounts>,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct RawRejection {
    pub id: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct RawSummary<'a> {
    pub market: String,
    #[serde(default)]
    pub mid: Option<Mid>,
    #[serde(default)]
    pub lot_mid_quote: Option<U128>,
    #[serde(default)]
    pub matched_lots: u32,
    #[serde(default)]
    pub base_volume: Option<U128>,
    #[serde(default)]
    pub quote_volume: Option<U128>,
    #[serde(default)]
    pub rebated: Option<U128>,
    #[serde(default)]
    pub venue_fee: Option<U128>,
    #[serde(default)]
    pub quotes: Vec<String>,
    /// Sub-batches of this market in `settle`, which is grouped by market in this order.
    #[serde(default)]
    pub sub_batches: u32,
    #[serde(default)]
    pub dropped: Vec<String>,
    #[serde(default, borrow)]
    pub skipped: Option<&'a RawValue>,
}

pub const MAX_QUOTES: usize = 4;
pub const MAX_DROPPED: usize = 16;
pub const MAX_TEXT: usize = 128;

impl RawSummary<'_> {
    /// Bounded copy for storage: the pool pays for batch records.
    pub fn into_summary(self) -> MarketSummary {
        let skipped = self.skipped.map(|raw| {
            let s = serde_json::from_slice::<String>(raw.get().as_bytes())
                .unwrap_or_else(|_| raw.get().to_string());
            clip(&s)
        });
        MarketSummary {
            market: clip(&self.market),
            mid: self.mid.map(|m| Mid {
                quote: clip(&m.quote),
                base: clip(&m.base),
            }),
            lot_mid_quote: self.lot_mid_quote,
            matched_lots: self.matched_lots,
            base_volume: self.base_volume.unwrap_or(U128(0)),
            quote_volume: self.quote_volume.unwrap_or(U128(0)),
            rebated: self.rebated.unwrap_or(U128(0)),
            venue_fee: self.venue_fee.unwrap_or(U128(0)),
            quotes: self
                .quotes
                .iter()
                .take(MAX_QUOTES)
                .map(|q| clip(q))
                .collect(),
            sub_batches: self.sub_batches,
            dropped: self
                .dropped
                .iter()
                .take(MAX_DROPPED)
                .map(|d| clip(d))
                .collect(),
            skipped,
        }
    }
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct KeysResult {
    pub v: u32,
    pub op: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub matcher_x25519: String,
    #[serde(default)]
    pub venue: String,
    #[serde(default)]
    pub result_key: String,
}

impl KeysResult {
    pub fn report(&self) -> KeysReport {
        KeysReport {
            matcher_x25519: clip(&self.matcher_x25519),
            venue: clip(&self.venue),
            result_key: clip(&self.result_key),
        }
    }
}

/// `execute_intents` args for one sub-batch: `{"signed":[...]}` with the
/// payloads exactly as the matcher produced them.
pub fn execute_intents_args(payloads: &[&RawValue]) -> Vec<u8> {
    let len = payloads.iter().map(|p| p.get().len() + 1).sum::<usize>() + 12;
    let mut out = String::with_capacity(len);
    out.push_str("{\"signed\":[");
    for (i, p) in payloads.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(p.get());
    }
    out.push_str("]}");
    out.into_bytes()
}

pub fn clip(s: &str) -> String {
    if s.len() <= MAX_TEXT {
        return s.to_string();
    }
    let mut end = MAX_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// `ed25519:<base58 32 bytes>` → raw key.
pub fn parse_ed25519_key(s: &str) -> Option<[u8; 32]> {
    decode_b58::<32>(s.strip_prefix("ed25519:")?)
}

/// `ed25519:<base58 64 bytes>` → raw signature.
pub fn parse_ed25519_sig(s: &str) -> Option<[u8; 64]> {
    decode_b58::<64>(s.strip_prefix("ed25519:")?)
}

fn decode_b58<const N: usize>(s: &str) -> Option<[u8; N]> {
    let v = bs58::decode(s).into_vec().ok()?;
    v.try_into().ok()
}

/// Canonical `ed25519:` form of a key string, or `None` if it is not one.
pub fn normalize_ed25519_key(s: &str) -> Option<String> {
    parse_ed25519_key(s).map(|k| format!("ed25519:{}", bs58::encode(k).into_string()))
}

/// An X25519 public key as 64 hex characters.
pub fn is_x25519_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}
