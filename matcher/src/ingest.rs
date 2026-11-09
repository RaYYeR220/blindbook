//! Opening and statically validating sealed orders (PROTOCOL.md §1).
//!
//! Everything here is pure: what still needs the live verifier (nonce unused,
//! salt valid, key registered) is returned as a list of checks for the batch
//! to run. Rejection reasons are deliberately generic; the detail never
//! leaves the enclave.

use std::collections::BTreeSet;

use blindbook_core::envelope::MatcherKey;
use blindbook_core::intent::{check_lot, LotSpec};
use blindbook_core::Side;
use serde::Deserialize;
use serde_json::Value;

use crate::book::{BookLot, BookOrder, OrderMeta};
use crate::input::{MarketCfg, OrderIn};
use crate::nonce;
use crate::util::{ed25519_bytes, hex, is_implicit_hex, unb64, unhex, valid_account_id};

/// Most lots one order may hold.
pub const MAX_LOTS_PER_ORDER: usize = 64;
/// Largest base64 envelope accepted: the 64 KiB bucket plus header and tag.
pub const MAX_SEALED_B64: usize = (65_536usize + 1 + 32 + 24 + 16).div_ceil(3) * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Invalid,
    Expired,
    UnsupportedMarket,
}

impl Reject {
    pub fn code(self) -> &'static str {
        match self {
            Reject::Invalid => "invalid",
            Reject::Expired => "expired",
            Reject::UnsupportedMarket => "unsupported_market",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderBody {
    v: u32,
    market: String,
    owner: String,
    side: Side,
    min_lots: u16,
    lots: Vec<LotBody>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LotBody {
    index: u16,
    limit_quote: String,
    intent: Value,
}

/// An order that passed every check the enclave can make on its own.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub order: BookOrder,
    /// Public keys of NEP-413 lots that must be registered for the owner on
    /// the verifier (none for an implicit account signing with its own key or
    /// for ERC-191 lots).
    pub keys: BTreeSet<String>,
    /// Hex salts of the lots' nonces.
    pub salts: BTreeSet<String>,
    /// Size of the order in the stored book.
    pub bytes: usize,
}

/// Open, parse and check one order. `horizon_ns` is the batch time plus the
/// deadline margin: an order whose earliest lot expires by then is refused.
pub fn open(
    key: &MatcherKey,
    order: &OrderIn,
    market: Option<&MarketCfg>,
    verifier: &str,
    horizon_ns: u64,
) -> Result<Candidate, Reject> {
    let market = market.ok_or(Reject::UnsupportedMarket)?;
    if order.sealed.len() > MAX_SEALED_B64 {
        return Err(Reject::Invalid);
    }
    let sealed = unb64(&order.sealed).ok_or(Reject::Invalid)?;
    let plain = key.open(&sealed).map_err(|_| Reject::Invalid)?;
    let body: OrderBody = serde_json::from_slice(&plain).map_err(|_| Reject::Invalid)?;

    let shape_ok = body.v == 1
        && body.market == market.id
        && body.market == order.market
        && valid_account_id(&body.owner)
        && (1..=MAX_LOTS_PER_ORDER).contains(&body.lots.len())
        && body.min_lots >= 1
        && body.min_lots as usize <= body.lots.len();
    if !shape_ok {
        return Err(Reject::Invalid);
    }

    let spec = LotSpec {
        verifier,
        owner: &body.owner,
        base: &market.base,
        quote: &market.quote,
        lot: market.lot,
        side: body.side,
    };
    let mut lots = Vec::with_capacity(body.lots.len());
    let mut indices = BTreeSet::new();
    let mut nonces = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut salts = BTreeSet::new();
    for l in &body.lots {
        let limit: u128 = l.limit_quote.parse().map_err(|_| Reject::Invalid)?;
        let checked = check_lot(&l.intent, &spec).map_err(|_| Reject::Invalid)?;
        if checked.limit_quote != limit
            || limit == 0
            || !indices.insert(l.index)
            || !nonces.insert(checked.nonce.clone())
        {
            return Err(Reject::Invalid);
        }
        let raw: [u8; 32] = unb64(&checked.nonce)
            .and_then(|n| n.try_into().ok())
            .ok_or(Reject::Invalid)?;
        let versioned = nonce::decode(&raw).ok_or(Reject::Invalid)?;
        if versioned.deadline_ns < 0 || (versioned.deadline_ns as u64) < checked.deadline_ns {
            return Err(Reject::Invalid);
        }
        salts.insert(hex(&versioned.salt));
        if let Some(pk) = &checked.public_key {
            let own_implicit_key =
                is_implicit_hex(&body.owner) && ed25519_bytes(pk).map(|b| hex(&b)) == Some(body.owner.clone());
            if !own_implicit_key {
                keys.insert(pk.clone());
            }
        }
        lots.push(BookLot {
            index: l.index,
            limit_quote: limit,
            nonce: checked.nonce,
            deadline_ns: checked.deadline_ns,
            intent: l.intent.clone(),
        });
    }
    let earliest = lots.iter().map(|l| l.deadline_ns).min().unwrap_or(0);
    if earliest <= horizon_ns {
        return Err(Reject::Expired);
    }
    lots.sort_by_key(|l| l.index);

    let order = BookOrder {
        meta: OrderMeta {
            id: order.id.clone(),
            market: market.id.clone(),
            owner: body.owner,
            side: body.side,
            min_lots: body.min_lots,
            seq: order.seq,
            base: market.base.clone(),
            quote: market.quote.clone(),
            lot: market.lot,
        },
        strikes: 0,
        lots,
    };
    let bytes = serde_json::to_vec(&order).map(|b| b.len()).unwrap_or(usize::MAX);
    Ok(Candidate {
        order,
        keys,
        salts,
        bytes,
    })
}

/// Salt bytes from their hex form.
pub fn salt_bytes(hex_salt: &str) -> Option<[u8; 4]> {
    unhex(hex_salt)?.try_into().ok()
}

/// The hex salt of a stored lot's nonce.
pub fn lot_salt(lot: &BookLot) -> Option<String> {
    let raw: [u8; 32] = unb64(&lot.nonce)?.try_into().ok()?;
    nonce::decode(&raw).map(|v| hex(&v.salt))
}
