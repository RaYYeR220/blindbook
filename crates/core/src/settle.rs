//! Turning a crossed batch into verifier-ready settlement.
//!
//! Two constraints of the NEAR Intents verifier shape this module:
//!
//! - **Protocol fee.** For every negative fungible-token delta `-X` in a
//!   `token_diff`, the verifier credits `ceil(X * fee / 10^6)` to its fee
//!   collector on top, and requires `sum(deltas) + sum(fees) == 0` per token.
//!   Traders sign exact amounts, so the venue signs the *closure*: the one
//!   delta per token that makes the batch balance.
//! - **Receipt log budget.** A receipt may emit at most 16 KiB of logs, which
//!   caps one `execute_intents` call at about a dozen intents. A batch is
//!   therefore split into sub-batches of at most `max_lots_per_side` sells and
//!   as many buys, each of which nets to zero on its own and settles
//!   atomically.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::{Batch, Market, MatchedLot, Rebate, Side};
use crate::Amount;

pub const FEE_DENOMINATOR: i128 = 1_000_000;

/// Signed token deltas keyed by asset id.
pub type Diff = BTreeMap<String, i128>;

/// Default cap that keeps one sub-batch (6 sells + 6 buys + the venue) under
/// the receipt log budget.
pub const DEFAULT_MAX_LOTS_PER_SIDE: usize = 6;

fn ceil_div(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0);
    if a >= 0 {
        (a + b - 1) / b
    } else {
        a / b
    }
}

fn floor_div(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0);
    if a >= 0 {
        a / b
    } else {
        -((-a + b - 1) / b)
    }
}

/// Protocol fee the verifier credits for one delta.
pub fn protocol_fee(delta: i128, fee_pips: u32) -> i128 {
    if delta >= 0 {
        0
    } else {
        ceil_div(-delta * fee_pips as i128, FEE_DENOMINATOR)
    }
}

/// The venue delta, per token, that makes `diffs` plus the venue's own delta
/// net to zero including protocol fees. Zero entries are omitted.
pub fn closure(diffs: &[Diff], fee_pips: u32) -> Diff {
    let keep = FEE_DENOMINATOR - fee_pips as i128;
    let mut supply: BTreeMap<String, i128> = BTreeMap::new();
    for d in diffs {
        for (token, &delta) in d {
            let s = if delta < 0 { ceil_div(delta * keep, FEE_DENOMINATOR) } else { delta };
            *supply.entry(token.clone()).or_default() += s;
        }
    }
    supply
        .into_iter()
        .filter_map(|(token, s)| {
            let c = -s;
            let c = if c < 0 { floor_div(c * FEE_DENOMINATOR, keep) } else { c };
            (c != 0).then_some((token, c))
        })
        .collect()
}

/// Net effect per token of `diffs` after protocol fees: zero for a balanced batch.
pub fn residual(diffs: &[Diff], fee_pips: u32) -> Diff {
    let mut out: Diff = BTreeMap::new();
    for d in diffs {
        for (token, &delta) in d {
            *out.entry(token.clone()).or_default() += delta + protocol_fee(delta, fee_pips);
        }
    }
    out.retain(|_, v| *v != 0);
    out
}

/// The diff a lot's signed intent commits to.
pub fn lot_diff(market: &Market, lot: &MatchedLot) -> Diff {
    let base = market.lot as i128;
    let quote = lot.limit_quote as i128;
    let mut d = Diff::new();
    match lot.side {
        Side::Sell => {
            d.insert(market.base.clone(), -base);
            d.insert(market.quote.clone(), quote);
        }
        Side::Buy => {
            d.insert(market.base.clone(), base);
            d.insert(market.quote.clone(), -quote);
        }
    }
    d
}

/// One atomic `execute_intents` call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubBatch {
    /// Signed trader intents, sells then buys.
    pub user_intents: Vec<Value>,
    /// The venue's closure delta.
    pub venue_diff: Diff,
    /// Price improvement transfers from the venue, one per owner.
    pub rebates: Vec<Rebate>,
    pub lots_per_side: u32,
    /// Venue balance change in quote: closure minus rebates. Positive is
    /// revenue, negative must be covered by venue inventory.
    pub venue_quote_net: i128,
    /// Venue balance change in base: the protocol-fee dust it pays.
    pub venue_base_net: i128,
}

impl SubBatch {
    /// The venue's intents for this sub-batch, in the verifier's JSON shape.
    /// They must be signed by the venue key and placed last in the call.
    pub fn venue_intents(&self, market: &Market) -> Vec<Value> {
        let mut out = Vec::with_capacity(1 + self.rebates.len());
        let diff: serde_json::Map<String, Value> =
            self.venue_diff.iter().map(|(k, v)| (k.clone(), Value::String(v.to_string()))).collect();
        out.push(json!({ "intent": "token_diff", "diff": diff }));
        for r in &self.rebates {
            out.push(json!({
                "intent": "transfer",
                "receiver_id": r.to,
                "tokens": { market.quote.clone(): r.amount.to_string() },
            }));
        }
        out
    }
}

/// Split a crossed batch into balanced sub-batches with their venue closures.
pub fn plan(batch: &Batch, market: &Market, fee_pips: u32, max_lots_per_side: usize) -> Vec<SubBatch> {
    assert_eq!(batch.sells.len(), batch.buys.len(), "engine must output equal lots per side");
    let step = max_lots_per_side.max(1);
    batch
        .sells
        .chunks(step)
        .zip(batch.buys.chunks(step))
        .map(|(sells, buys)| {
            let lots: Vec<&MatchedLot> = sells.iter().chain(buys.iter()).collect();
            let diffs: Vec<Diff> = lots.iter().map(|l| lot_diff(market, l)).collect();
            let venue_diff = closure(&diffs, fee_pips);
            let mut by_owner: BTreeMap<String, Amount> = BTreeMap::new();
            for l in &lots {
                if l.rebate > 0 {
                    *by_owner.entry(l.owner.clone()).or_default() += l.rebate;
                }
            }
            let rebates: Vec<Rebate> = by_owner.into_iter().map(|(to, amount)| Rebate { to, amount }).collect();
            let rebate_total: i128 = rebates.iter().map(|r| r.amount as i128).sum();
            SubBatch {
                user_intents: lots.iter().map(|l| l.intent.clone()).collect(),
                venue_quote_net: venue_diff.get(&market.quote).copied().unwrap_or(0) - rebate_total,
                venue_base_net: venue_diff.get(&market.base).copied().unwrap_or(0),
                venue_diff,
                rebates,
                lots_per_side: sells.len() as u32,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{cross, Lot, Order, Params};
    use crate::price::Price;

    const E24: i128 = 1_000_000_000_000_000_000_000_000;

    fn diff(pairs: &[(&str, i128)]) -> Diff {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn closure_matches_the_verifier_on_testnet() {
        // Settled on testnet with fee = 1 pip (tx 8f4ubY6LPvmNXDuBbNoBgr6sQcTmji2jn12av4qCVqAZ).
        let seller = diff(&[("n", -100 * E24), ("u", 250_000_000)]);
        let buyer = diff(&[("n", 100 * E24), ("u", -260_000_000)]);
        let venue = closure(&[seller.clone(), buyer.clone()], 1);
        assert_eq!(venue["n"], -100_000_100_000_100_000_101);
        assert_eq!(venue["u"], 9_999_740);
        assert!(residual(&[seller, buyer, venue], 1).is_empty());
    }

    #[test]
    fn closure_without_fee_is_the_negated_sum() {
        let a = diff(&[("n", -10), ("u", 25)]);
        let b = diff(&[("n", 10), ("u", -26)]);
        let venue = closure(&[a, b], 0);
        assert_eq!(venue, diff(&[("u", 1)]));
    }

    #[test]
    fn closure_balances_random_batches_at_several_fees() {
        let mut x: u64 = 0xDEADBEEF;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for fee in [0u32, 1, 7, 100, 3_000] {
            for _ in 0..2_000 {
                let n = (next() % 6 + 1) as usize;
                let mut diffs = Vec::new();
                for _ in 0..n {
                    let lot = (next() % 1_000 + 1) as i128 * E24 / 10;
                    let q = (next() % 10_000_000 + 1) as i128;
                    if next() % 2 == 0 {
                        diffs.push(diff(&[("n", -lot), ("u", q)]));
                    } else {
                        diffs.push(diff(&[("n", lot), ("u", -q)]));
                    }
                }
                let venue = closure(&diffs, fee);
                diffs.push(venue);
                assert!(residual(&diffs, fee).is_empty(), "fee {fee}: {:?}", residual(&diffs, fee));
            }
        }
    }

    fn market() -> Market {
        Market { base: "nep141:wrap.near".into(), quote: "nep141:usdc".into(), lot: (E24 / 10) as Amount }
    }

    fn order(id: &str, owner: &str, side: Side, lots: usize, limit: Amount, seq: u64) -> Order {
        Order {
            id: id.into(),
            owner: owner.into(),
            side,
            lots: (0..lots)
                .map(|i| Lot { index: i as u16, limit_quote: limit, intent: json!({ "o": id, "l": i }) })
                .collect(),
            min_lots: 1,
            deadline_ns: u64::MAX,
            seq,
        }
    }

    fn crossed(book: &[Order]) -> Batch {
        // mid 3.60 USDC/NEAR -> 0.36 USDC per 0.1 NEAR lot
        let bid = Price::new(3_590_000, E24 as Amount).unwrap();
        let ask = Price::new(3_610_000, E24 as Amount).unwrap();
        cross(&market(), book, bid, ask, 0, &Params { fee_bps: 5, max_spread_bps: 150, deadline_margin_ns: 0 }).unwrap()
    }

    #[test]
    fn plan_splits_into_balanced_sub_batches() {
        let book = vec![
            order("s1", "a.near", Side::Sell, 9, 350_000, 1),
            order("s2", "b.near", Side::Sell, 5, 355_000, 2),
            order("b1", "c.near", Side::Buy, 8, 370_000, 3),
            order("b2", "d.near", Side::Buy, 6, 365_000, 4),
        ];
        let batch = crossed(&book);
        assert_eq!(batch.matched_lots, 14);
        let subs = plan(&batch, &market(), 1, 6);
        assert_eq!(subs.iter().map(|s| s.lots_per_side).collect::<Vec<_>>(), vec![6, 6, 2]);
        let m = market();
        for s in &subs {
            assert_eq!(s.user_intents.len(), 2 * s.lots_per_side as usize);
            assert!(s.user_intents.len() + 1 <= 13, "sub-batch must fit the log budget");
            // all deltas of the sub-batch, including the venue, net to zero after fees
            let mut diffs: Vec<Diff> = Vec::new();
            let lots_in: Vec<&MatchedLot> = batch
                .sells
                .iter()
                .chain(batch.buys.iter())
                .filter(|l| s.user_intents.contains(&l.intent))
                .collect();
            for l in &lots_in {
                diffs.push(lot_diff(&m, l));
            }
            diffs.push(s.venue_diff.clone());
            assert!(residual(&diffs, 1).is_empty());
            // venue keeps a non-negative quote margin and pays only base dust
            assert!(s.venue_quote_net >= 0, "venue quote {}", s.venue_quote_net);
            assert!(s.venue_base_net <= 0 && s.venue_base_net > -(E24 / 1_000));
        }
        let rebates: Amount = subs.iter().flat_map(|s| s.rebates.iter()).map(|r| r.amount).sum();
        let batch_rebates: Amount = batch.rebates.iter().map(|r| r.amount).sum();
        assert_eq!(rebates, batch_rebates);
    }

    #[test]
    fn venue_intents_are_closure_then_rebates() {
        let book = vec![order("s1", "a.near", Side::Sell, 1, 350_000, 1), order("b1", "c.near", Side::Buy, 1, 370_000, 2)];
        let subs = plan(&crossed(&book), &market(), 1, 6);
        let intents = subs[0].venue_intents(&market());
        assert_eq!(intents[0]["intent"], "token_diff");
        assert_eq!(intents.len(), 3);
        assert_eq!(intents[1]["intent"], "transfer");
        assert_eq!(intents[1]["receiver_id"], "a.near");
        // 0.36 mid - 0.35 limit = 0.01 improvement, minus 5 bps of 0.36 = 0.00018
        assert_eq!(intents[1]["tokens"]["nep141:usdc"], "9820");
    }
}
