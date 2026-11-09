//! The public `input_data` the pool sends (PROTOCOL.md §3.1).

use blindbook_core::settle::DEFAULT_MAX_LOTS_PER_SIDE;
use serde::Deserialize;

use crate::util::valid_account_id;

/// Most markets one batch prices: two quotes each.
pub const MAX_MARKETS: usize = 8;
/// Most orders one input may carry; the rest wait for the next batch.
pub const MAX_INPUT_ORDERS: usize = 64;
/// Upper bound of `params.max_sub_batches`.
pub const MAX_SUB_BATCHES: u32 = 16;
/// Default freshness window of a 1Click quote.
pub const DEFAULT_QUOTE_MAX_AGE_S: u64 = 120;

fn u128_from_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
    let s = String::deserialize(d)?;
    s.parse::<u128>().map_err(serde::de::Error::custom)
}

fn u64_from_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    s.parse::<u64>().map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Keys,
    Batch,
}

/// Just enough of the input to dispatch, and to answer `keys`.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub op: Op,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reference {
    pub base: String,
    pub quote: String,
    #[serde(deserialize_with = "u128_from_str")]
    pub probe_base: u128,
    #[serde(deserialize_with = "u128_from_str")]
    pub probe_quote: u128,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketCfg {
    pub id: String,
    pub base: String,
    pub quote: String,
    #[serde(deserialize_with = "u128_from_str")]
    pub lot: u128,
    pub reference: Reference,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Params {
    /// Venue fee per side, in bps of the midpoint notional, taken only out of
    /// price improvement.
    pub fee_bps: u32,
    /// Skip a market whose reference spread is wider than this.
    pub max_spread_bps: u32,
    /// Lots per side in one sub-batch (one `execute_intents` call).
    pub max_lots_per_side: u32,
    /// Sub-batches per result, over all markets.
    pub max_sub_batches: u32,
    /// Orders expiring within this window neither enter the book nor cross.
    pub deadline_margin_s: u64,
    /// Freshness window of a 1Click quote against the enclave clock; `0`
    /// disables the check. Absent: 120 s.
    #[serde(default)]
    pub quote_max_age_s: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderIn {
    pub id: String,
    pub seq: u64,
    pub market: String,
    /// Base64 of the envelope.
    pub sealed: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchInput {
    pub v: u32,
    pub op: Op,
    pub network: String,
    pub verifier: String,
    /// Verifier account the venue signs as; the `venue` host key must be one
    /// of its registered keys.
    pub venue_account: String,
    #[serde(deserialize_with = "u64_from_str")]
    pub now_ns: u64,
    pub markets: Vec<MarketCfg>,
    pub params: Params,
    #[serde(default)]
    pub orders: Vec<OrderIn>,
}

impl BatchInput {
    /// Structural checks; anything off fails the whole run closed.
    pub fn validate(&self, expected_network: &str) -> Result<(), &'static str> {
        if self.v != 1 || self.op != Op::Batch {
            return Err("bad_input");
        }
        if self.network != expected_network {
            return Err("network_mismatch");
        }
        if !valid_account_id(&self.verifier) || !valid_account_id(&self.venue_account) {
            return Err("bad_input");
        }
        if self.markets.len() > MAX_MARKETS || self.orders.len() > MAX_INPUT_ORDERS {
            return Err("bad_input");
        }
        let p = &self.params;
        let lots_ok = (1..=DEFAULT_MAX_LOTS_PER_SIDE as u32).contains(&p.max_lots_per_side);
        let subs_ok = (1..=MAX_SUB_BATCHES).contains(&p.max_sub_batches);
        if p.fee_bps > 1_000 || p.max_spread_bps > 10_000 || !lots_ok || !subs_ok || p.deadline_margin_s > 86_400 {
            return Err("bad_input");
        }
        let mut ids = std::collections::BTreeSet::new();
        for m in &self.markets {
            let r = &m.reference;
            let ok = !m.id.is_empty()
                && m.id.len() <= 64
                && ids.insert(m.id.as_str())
                && m.base != m.quote
                && !m.base.is_empty()
                && !m.quote.is_empty()
                && m.lot > 0
                && m.lot <= i128::MAX as u128 / 1_000
                && r.base != r.quote
                && r.probe_base > 0
                && r.probe_quote > 0;
            if !ok {
                return Err("bad_input");
            }
        }
        if self.orders.iter().any(|o| o.id.is_empty() || o.id.len() > 64) {
            return Err("bad_input");
        }
        Ok(())
    }

    pub fn market(&self, id: &str) -> Option<&MarketCfg> {
        self.markets.iter().find(|m| m.id == id)
    }

    pub fn quote_max_age_s(&self) -> Option<u64> {
        match self.params.quote_max_age_s {
            None => Some(DEFAULT_QUOTE_MAX_AGE_S),
            Some(0) => None,
            Some(s) => Some(s),
        }
    }
}
