//! Blindbook core: the batch crossing engine.
//!
//! Orders arrive as sealed bundles of pre-signed lots. Every lot is one
//! `token_diff` intent at the trader's own limit for exactly `market.lot` base
//! units. At each batch the engine:
//!
//! 1. takes a reference midpoint from a two-sided quote,
//! 2. keeps the lots whose limit is compatible with that midpoint,
//! 3. allocates an equal number of lots on each side by time priority,
//!    honouring each order's minimum fill and blocking self-trades,
//! 4. returns the signed intents to settle plus the price improvement each
//!    trader is owed, so the venue can hand it back in the same atomic batch.
//!
//! Because every lot is signed at the trader's limit, the settlement layer
//! (the NEAR Intents verifier) never lets a lot execute at a worse price,
//! whatever this engine outputs. The engine only decides *which* lots cross
//! and how the surplus between the two limits is split.

pub mod engine;
pub mod price;

pub use engine::{cross, Batch, Fill, Lot, Market, Order, Params, Rebate, Side, SkipReason};
pub use price::{midpoint, Price, PriceError};

/// Token amounts in the token's smallest unit.
pub type Amount = u128;

/// Serialize `u128` as a decimal string, the convention used by NEAR JSON.
pub mod u128_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<u128>().map_err(serde::de::Error::custom)
    }
}
