use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::price::{bps_of, Price, PriceError};
use crate::Amount;

/// A trading pair with a fixed lot size.
///
/// Lots are what make atomic settlement possible: the verifier requires every
/// token to net to zero across a batch, and equal-size lots on both sides
/// guarantee the base token always does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Market {
    /// Asset id of the base token, e.g. `nep141:wrap.near`.
    pub base: String,
    /// Asset id of the quote token.
    pub quote: String,
    /// Base units per lot.
    #[serde(with = "crate::u128_str")]
    pub lot: Amount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Gives base, receives quote.
    Sell,
    /// Gives quote, receives base.
    Buy,
}

/// One pre-signed lot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub index: u16,
    /// For a sell lot: the minimum quote received. For a buy lot: the maximum
    /// quote paid. This is the exact amount inside the signed intent.
    #[serde(with = "crate::u128_str")]
    pub limit_quote: Amount,
    /// The signed `token_diff` intent, passed through untouched.
    pub intent: Value,
}

/// A resting order: a bundle of lots from one trader on one side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Order {
    /// Commitment id of the sealed order.
    pub id: String,
    /// Verifier account that signed the lots.
    pub owner: String,
    pub side: Side,
    pub lots: Vec<Lot>,
    /// Smallest number of lots this order accepts in one batch.
    pub min_lots: u16,
    /// Earliest deadline among the signed lots, in nanoseconds.
    pub deadline_ns: u64,
    /// Arrival sequence; lower crosses first.
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    /// Venue fee per side, in basis points of the midpoint notional. Only ever
    /// taken out of a trader's price improvement, never out of their limit.
    pub fee_bps: u32,
    /// The batch does not cross if the reference spread is wider than this.
    pub max_spread_bps: u32,
    /// Lots expiring within this window are not used, so settlement cannot
    /// race a deadline.
    pub deadline_margin_ns: u64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            fee_bps: 5,
            max_spread_bps: 150,
            deadline_margin_ns: 60_000_000_000,
        }
    }
}

/// Lots of one order that crossed in this batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    pub order_id: String,
    pub owner: String,
    pub side: Side,
    pub lot_indices: Vec<u16>,
    /// Sum of the signed limits of the filled lots.
    #[serde(with = "crate::u128_str")]
    pub limit_quote: Amount,
    /// What the filled lots are worth at the midpoint.
    #[serde(with = "crate::u128_str")]
    pub mid_quote: Amount,
    /// Price improvement handed back to the trader.
    #[serde(with = "crate::u128_str")]
    pub rebate: Amount,
    /// Venue fee kept from this order's improvement.
    #[serde(with = "crate::u128_str")]
    pub fee: Amount,
}

/// A quote transfer from the venue account to a trader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rebate {
    pub to: String,
    #[serde(with = "crate::u128_str")]
    pub amount: Amount,
}

/// One lot that crossed, with everything settlement needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchedLot {
    pub order_id: String,
    pub owner: String,
    pub side: Side,
    pub index: u16,
    #[serde(with = "crate::u128_str")]
    pub limit_quote: Amount,
    #[serde(with = "crate::u128_str")]
    pub rebate: Amount,
    #[serde(with = "crate::u128_str")]
    pub fee: Amount,
    pub intent: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SkipReason {
    BadReference { detail: String },
    NoCross,
}

/// Output of one batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub mid: Option<Price>,
    /// Quote value of one lot at the midpoint.
    #[serde(with = "crate::u128_str")]
    pub lot_mid_quote: Amount,
    pub matched_lots: u32,
    #[serde(with = "crate::u128_str")]
    pub base_volume: Amount,
    #[serde(with = "crate::u128_str")]
    pub quote_volume: Amount,
    pub fills: Vec<Fill>,
    /// Quote the venue must absorb with its own `token_diff` so the batch
    /// nets to zero: the gap between the buyers' and sellers' limits.
    #[serde(with = "crate::u128_str")]
    pub surplus: Amount,
    /// Part of the surplus the venue keeps.
    #[serde(with = "crate::u128_str")]
    pub fee: Amount,
    /// Part of the surplus returned to traders, one entry per owner.
    pub rebates: Vec<Rebate>,
    /// Signed lot intents to submit, sells first then buys, in allocation order.
    pub intents: Vec<Value>,
    /// Matched sell lots, in allocation order.
    pub sells: Vec<MatchedLot>,
    /// Matched buy lots, in allocation order. Same length as `sells`.
    pub buys: Vec<MatchedLot>,
    pub skipped: Option<SkipReason>,
}

impl Batch {
    fn empty(mid: Option<Price>, lot_mid_quote: Amount, skipped: SkipReason) -> Self {
        Self {
            mid,
            lot_mid_quote,
            matched_lots: 0,
            base_volume: 0,
            quote_volume: 0,
            fills: vec![],
            surplus: 0,
            fee: 0,
            rebates: vec![],
            intents: vec![],
            sells: vec![],
            buys: vec![],
            skipped: Some(skipped),
        }
    }
}

struct Candidate<'a> {
    order: &'a Order,
    /// Eligible lots in index order.
    lots: Vec<&'a Lot>,
}

/// Run one batch.
///
/// `bid` and `ask` are the reference market (quote per base when selling and
/// when buying base). `now_ns` is the batch time.
pub fn cross(
    market: &Market,
    book: &[Order],
    bid: Price,
    ask: Price,
    now_ns: u64,
    params: &Params,
) -> Result<Batch, PriceError> {
    let mid = match crate::price::midpoint(bid, ask, params.max_spread_bps) {
        Ok(m) => m,
        Err(e @ (PriceError::Crossed { .. } | PriceError::SpreadTooWide { .. })) => {
            return Ok(Batch::empty(
                None,
                0,
                SkipReason::BadReference { detail: e.to_string() },
            ))
        }
        Err(e) => return Err(e),
    };
    let lot_mid = mid.quote_for(market.lot)?;

    let horizon = now_ns.saturating_add(params.deadline_margin_ns);
    let mut sells = candidates(book, Side::Sell, lot_mid, horizon);
    let mut buys = candidates(book, Side::Buy, lot_mid, horizon);
    block_self_trades(&mut sells, &mut buys);

    let (sell_alloc, buy_alloc) = allocate_both(&sells, &buys);
    let k = total(&sell_alloc);
    if k == 0 {
        return Ok(Batch::empty(Some(mid), lot_mid, SkipReason::NoCross));
    }

    let mut fills = Vec::new();
    let mut intents = Vec::new();
    let mut surplus: Amount = 0;
    let mut fee_total: Amount = 0;
    let mut rebate_by_owner: BTreeMap<String, Amount> = BTreeMap::new();
    let mut matched_sells = Vec::with_capacity(k);
    let mut matched_buys = Vec::with_capacity(k);
    let lot_fee_cap = bps_of(lot_mid, params.fee_bps)?;

    for (cands, alloc) in [(&sells, &sell_alloc), (&buys, &buy_alloc)] {
        for (ci, take) in alloc {
            let c = &cands[*ci];
            let mut fill = Fill {
                order_id: c.order.id.clone(),
                owner: c.order.owner.clone(),
                side: c.order.side,
                lot_indices: Vec::with_capacity(*take),
                limit_quote: 0,
                mid_quote: 0,
                rebate: 0,
                fee: 0,
            };
            for lot in c.lots.iter().take(*take) {
                let improvement = match c.order.side {
                    Side::Sell => lot_mid - lot.limit_quote,
                    Side::Buy => lot.limit_quote - lot_mid,
                };
                let fee = improvement.min(lot_fee_cap);
                fill.lot_indices.push(lot.index);
                fill.limit_quote += lot.limit_quote;
                fill.mid_quote += lot_mid;
                fill.rebate += improvement - fee;
                fill.fee += fee;
                surplus += improvement;
                intents.push(lot.intent.clone());
                let matched = MatchedLot {
                    order_id: c.order.id.clone(),
                    owner: c.order.owner.clone(),
                    side: c.order.side,
                    index: lot.index,
                    limit_quote: lot.limit_quote,
                    rebate: improvement - fee,
                    fee,
                    intent: lot.intent.clone(),
                };
                match c.order.side {
                    Side::Sell => matched_sells.push(matched),
                    Side::Buy => matched_buys.push(matched),
                }
            }
            fee_total += fill.fee;
            if fill.rebate > 0 {
                *rebate_by_owner.entry(fill.owner.clone()).or_default() += fill.rebate;
            }
            fills.push(fill);
        }
    }

    let rebates = rebate_by_owner
        .into_iter()
        .map(|(to, amount)| Rebate { to, amount })
        .collect();

    Ok(Batch {
        mid: Some(mid),
        lot_mid_quote: lot_mid,
        matched_lots: k as u32,
        base_volume: market.lot * k as Amount,
        quote_volume: lot_mid * k as Amount,
        fills,
        surplus,
        fee: fee_total,
        rebates,
        intents,
        sells: matched_sells,
        buys: matched_buys,
        skipped: None,
    })
}

fn candidates<'a>(book: &'a [Order], side: Side, lot_mid: Amount, horizon: u64) -> Vec<Candidate<'a>> {
    let mut out: Vec<Candidate<'a>> = book
        .iter()
        .filter(|o| o.side == side && o.deadline_ns > horizon && o.min_lots >= 1)
        .map(|o| {
            let mut lots: Vec<&Lot> = o
                .lots
                .iter()
                .filter(|l| match side {
                    Side::Sell => l.limit_quote <= lot_mid,
                    Side::Buy => l.limit_quote >= lot_mid,
                })
                .collect();
            lots.sort_by_key(|l| l.index);
            Candidate { order: o, lots }
        })
        .filter(|c| c.lots.len() >= c.order.min_lots as usize)
        .collect();
    out.sort_by_key(|c| (c.order.seq, c.order.id.clone()));
    out
}

/// A trader with live interest on both sides never trades with themselves:
/// they keep only one side for this batch. The kept side is the one the rest
/// of the book needs (the side facing the larger external imbalance), so a
/// two-sided liquidity provider meets incoming flow in either direction. On a
/// tie, the side they entered first is kept.
fn block_self_trades(sells: &mut Vec<Candidate>, buys: &mut Vec<Candidate>) {
    let first_seq = |cands: &Vec<Candidate>, owner: &str| {
        cands.iter().filter(|c| c.order.owner == owner).map(|c| c.order.seq).min()
    };
    let sell_owners: BTreeSet<String> = sells.iter().map(|c| c.order.owner.clone()).collect();
    let both: BTreeSet<String> =
        buys.iter().map(|c| c.order.owner.clone()).filter(|o| sell_owners.contains(o)).collect();
    if both.is_empty() {
        return;
    }
    let external = |cands: &Vec<Candidate>| -> usize {
        cands.iter().filter(|c| !both.contains(&c.order.owner)).map(|c| c.lots.len()).sum()
    };
    let (ext_sell, ext_buy) = (external(sells), external(buys));
    for owner in &both {
        let keep_sells = if ext_buy > ext_sell {
            true
        } else if ext_sell > ext_buy {
            false
        } else {
            first_seq(sells, owner).unwrap_or(u64::MAX) <= first_seq(buys, owner).unwrap_or(u64::MAX)
        };
        if keep_sells {
            buys.retain(|c| &c.order.owner != owner);
        } else {
            sells.retain(|c| &c.order.owner != owner);
        }
    }
}

/// Greedy time-priority allocation of exactly up to `target` lots.
fn allocate(cands: &[Candidate], target: usize) -> Vec<(usize, usize)> {
    let mut remaining = target;
    let mut out = Vec::new();
    for (i, c) in cands.iter().enumerate() {
        if remaining == 0 {
            break;
        }
        let take = c.lots.len().min(remaining);
        if take < c.order.min_lots as usize {
            continue;
        }
        out.push((i, take));
        remaining -= take;
    }
    out
}

fn total(alloc: &[(usize, usize)]) -> usize {
    alloc.iter().map(|(_, t)| t).sum()
}

/// Find the largest lot count both sides can fill exactly.
fn allocate_both(sells: &[Candidate], buys: &[Candidate]) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let avail = |c: &[Candidate]| c.iter().map(|c| c.lots.len()).sum::<usize>();
    let mut k = avail(sells).min(avail(buys));
    while k > 0 {
        let s = allocate(sells, k);
        let b = allocate(buys, k);
        let (ts, tb) = (total(&s), total(&b));
        if ts == k && tb == k {
            return (s, b);
        }
        let next = ts.min(tb);
        k = if next < k { next } else { k - 1 };
    }
    (vec![], vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NEAR: Amount = 1_000_000_000_000_000_000_000_000;
    const USDC: Amount = 1_000_000;

    fn market() -> Market {
        Market {
            base: "nep141:wrap.near".into(),
            quote: "nep141:usdc".into(),
            lot: 10 * NEAR,
        }
    }

    /// Reference market 3.59 / 3.61 -> mid 3.60, one lot = 36 USDC at mid.
    fn reference() -> (Price, Price) {
        (Price::new(3_590_000, NEAR).unwrap(), Price::new(3_610_000, NEAR).unwrap())
    }

    fn order(id: &str, owner: &str, side: Side, limits_usdc_milli: &[u128], seq: u64) -> Order {
        Order {
            id: id.into(),
            owner: owner.into(),
            side,
            lots: limits_usdc_milli
                .iter()
                .enumerate()
                .map(|(i, l)| Lot {
                    index: i as u16,
                    limit_quote: l * USDC / 1000,
                    intent: json!({ "order": id, "lot": i }),
                })
                .collect(),
            min_lots: 1,
            deadline_ns: 10_000_000_000_000,
            seq,
        }
    }

    fn params() -> Params {
        Params { fee_bps: 5, max_spread_bps: 150, deadline_margin_ns: 0 }
    }

    fn run(book: &[Order]) -> Batch {
        let (bid, ask) = reference();
        cross(&market(), book, bid, ask, 0, &params()).unwrap()
    }

    #[test]
    fn crosses_one_lot_at_the_midpoint_and_returns_improvement() {
        // seller wants >= 35.00 for 10 NEAR, buyer pays <= 37.00
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_000], 1),
            order("b1", "bob.near", Side::Buy, &[37_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 1);
        assert_eq!(b.lot_mid_quote, 36 * USDC);
        assert_eq!(b.base_volume, 10 * NEAR);
        // surplus = 37 - 35 = 2 USDC
        assert_eq!(b.surplus, 2 * USDC);
        // fee cap = 5 bps of 36 USDC = 0.018 USDC per side
        assert_eq!(b.fee, 2 * 18_000);
        let seller = &b.fills[0];
        assert_eq!(seller.rebate, USDC - 18_000);
        let buyer = &b.fills[1];
        assert_eq!(buyer.rebate, USDC - 18_000);
        // the batch conserves value: limits + surplus balance out
        assert_eq!(seller.limit_quote + seller.rebate + seller.fee, 36 * USDC);
        assert_eq!(buyer.limit_quote - buyer.rebate - buyer.fee, 36 * USDC);
        assert_eq!(b.intents.len(), 2);
        assert!(b.skipped.is_none());
    }

    #[test]
    fn effective_price_is_mid_minus_fee_for_both_sides() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[30_000, 30_000], 1),
            order("b1", "bob.near", Side::Buy, &[40_000, 40_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 2);
        let s = &b.fills[0];
        let bu = &b.fills[1];
        // seller nets mid - fee, buyer pays mid + fee
        assert_eq!(s.limit_quote + s.rebate, 72 * USDC - 2 * 18_000);
        assert_eq!(bu.limit_quote - bu.rebate, 72 * USDC + 2 * 18_000);
    }

    #[test]
    fn surplus_equals_rebates_plus_fee() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_500, 35_900], 1),
            order("s2", "carol.near", Side::Sell, &[34_000], 3),
            order("b1", "bob.near", Side::Buy, &[36_010, 36_500, 38_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 3);
        let rebates: Amount = b.rebates.iter().map(|r| r.amount).sum();
        assert_eq!(rebates + b.fee, b.surplus);
    }

    #[test]
    fn fee_never_exceeds_improvement() {
        // seller limit is 1 micro below mid: improvement 1 unit, fee cap 18_000
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_999], 1),
            order("b1", "bob.near", Side::Buy, &[36_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 1);
        assert_eq!(b.fills[0].fee, 1_000);
        assert_eq!(b.fills[0].rebate, 0);
        // buyer limit equals mid: zero improvement, zero fee
        assert_eq!(b.fills[1].fee, 0);
        assert_eq!(b.fills[1].rebate, 0);
        assert!(b.rebates.is_empty());
    }

    #[test]
    fn limits_on_the_wrong_side_of_mid_do_not_cross() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[36_100], 1), // wants more than mid
            order("b1", "bob.near", Side::Buy, &[37_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 0);
        assert_eq!(b.skipped, Some(SkipReason::NoCross));
    }

    #[test]
    fn time_priority_on_the_heavy_side() {
        let book = vec![
            order("s_late", "carol.near", Side::Sell, &[35_000], 9),
            order("s_early", "alice.near", Side::Sell, &[35_500], 1),
            order("b1", "bob.near", Side::Buy, &[37_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 1);
        assert_eq!(b.fills[0].order_id, "s_early");
    }

    #[test]
    fn partial_fill_takes_the_lowest_lot_indices() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_000, 35_000, 35_000, 35_000], 1),
            order("b1", "bob.near", Side::Buy, &[37_000, 37_000], 2),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 2);
        assert_eq!(b.fills[0].lot_indices, vec![0, 1]);
    }

    #[test]
    fn min_lots_is_all_or_nothing_per_batch() {
        let mut big = order("s1", "alice.near", Side::Sell, &[35_000, 35_000, 35_000], 1);
        big.min_lots = 3;
        let book = vec![big, order("b1", "bob.near", Side::Buy, &[37_000, 37_000], 2)];
        let b = run(&book);
        // seller needs 3 lots at once, only 2 lots of demand
        assert_eq!(b.matched_lots, 0);
    }

    #[test]
    fn min_lots_skips_to_the_next_order_that_fits() {
        let mut big = order("s_big", "alice.near", Side::Sell, &[35_000, 35_000, 35_000], 1);
        big.min_lots = 3;
        let book = vec![
            big,
            order("s_small", "carol.near", Side::Sell, &[35_000, 35_000], 2),
            order("b1", "bob.near", Side::Buy, &[37_000, 37_000], 3),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 2);
        assert_eq!(b.fills[0].order_id, "s_small");
    }

    #[test]
    fn self_trade_is_blocked_keeping_the_earlier_side() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_000], 1),
            order("b_self", "alice.near", Side::Buy, &[37_000], 2),
            order("b_other", "bob.near", Side::Buy, &[37_000], 3),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 1);
        assert!(b.fills.iter().all(|f| f.order_id != "b_self"));
        assert!(b.fills.iter().any(|f| f.order_id == "b_other"));
    }

    #[test]
    fn two_sided_maker_fills_the_side_the_flow_needs() {
        // the maker rests on both sides; a user sells after it
        let book = vec![
            order("mm_ask", "maker.near", Side::Sell, &[35_500, 35_500], 1),
            order("mm_bid", "maker.near", Side::Buy, &[36_500, 36_500], 2),
            order("user_sell", "alice.near", Side::Sell, &[35_900], 3),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 1);
        let ids: Vec<&str> = b.fills.iter().map(|f| f.order_id.as_str()).collect();
        assert_eq!(ids, vec!["user_sell", "mm_bid"]);
    }

    #[test]
    fn two_sided_maker_meets_incoming_buys_too() {
        let book = vec![
            order("mm_ask", "maker.near", Side::Sell, &[35_500, 35_500], 1),
            order("mm_bid", "maker.near", Side::Buy, &[36_500, 36_500], 2),
            order("user_buy", "bob.near", Side::Buy, &[36_100, 36_100], 3),
        ];
        let b = run(&book);
        assert_eq!(b.matched_lots, 2);
        let ids: Vec<&str> = b.fills.iter().map(|f| f.order_id.as_str()).collect();
        assert_eq!(ids, vec!["mm_ask", "user_buy"]);
    }

    #[test]
    fn a_maker_alone_never_trades_with_itself() {
        let book = vec![
            order("mm_ask", "maker.near", Side::Sell, &[35_500], 1),
            order("mm_bid", "maker.near", Side::Buy, &[36_500], 2),
        ];
        assert_eq!(run(&book).matched_lots, 0);
    }

    #[test]
    fn expiring_orders_are_left_out() {
        let mut s = order("s1", "alice.near", Side::Sell, &[35_000], 1);
        s.deadline_ns = 5;
        let book = vec![s, order("b1", "bob.near", Side::Buy, &[37_000], 2)];
        let (bid, ask) = reference();
        let p = Params { deadline_margin_ns: 10, ..params() };
        let b = cross(&market(), &book, bid, ask, 0, &p).unwrap();
        assert_eq!(b.matched_lots, 0);
    }

    #[test]
    fn wide_reference_spread_skips_the_batch() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_000], 1),
            order("b1", "bob.near", Side::Buy, &[37_000], 2),
        ];
        let bid = Price::new(3_400_000, NEAR).unwrap();
        let ask = Price::new(3_800_000, NEAR).unwrap();
        let b = cross(&market(), &book, bid, ask, 0, &params()).unwrap();
        assert_eq!(b.matched_lots, 0);
        assert!(matches!(b.skipped, Some(SkipReason::BadReference { .. })));
    }

    #[test]
    fn base_nets_to_zero_across_many_orders() {
        let book = vec![
            order("s1", "a.near", Side::Sell, &[35_000, 35_100, 35_200], 1),
            order("s2", "b.near", Side::Sell, &[35_900], 2),
            order("s3", "c.near", Side::Sell, &[36_500], 3), // not eligible
            order("b1", "d.near", Side::Buy, &[36_200, 36_200], 4),
            order("b2", "e.near", Side::Buy, &[36_000, 39_000, 35_000], 5), // last lot not eligible
        ];
        let b = run(&book);
        let sold: usize = b.fills.iter().filter(|f| f.side == Side::Sell).map(|f| f.lot_indices.len()).sum();
        let bought: usize = b.fills.iter().filter(|f| f.side == Side::Buy).map(|f| f.lot_indices.len()).sum();
        assert_eq!(sold, bought);
        assert_eq!(b.matched_lots as usize, sold);
        assert_eq!(b.matched_lots, 4);
        let rebates: Amount = b.rebates.iter().map(|r| r.amount).sum();
        assert_eq!(rebates + b.fee, b.surplus);
    }

    #[test]
    fn rebates_are_aggregated_per_owner() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_000], 1),
            order("s2", "alice.near", Side::Sell, &[35_000], 2),
            order("b1", "bob.near", Side::Buy, &[37_000, 37_000], 3),
        ];
        let b = run(&book);
        assert_eq!(b.rebates.len(), 2);
        let alice = b.rebates.iter().find(|r| r.to == "alice.near").unwrap();
        assert_eq!(alice.amount, 2 * (USDC - 18_000));
    }

    #[test]
    fn batch_serializes_amounts_as_strings() {
        let book = vec![
            order("s1", "alice.near", Side::Sell, &[35_000], 1),
            order("b1", "bob.near", Side::Buy, &[37_000], 2),
        ];
        let v = serde_json::to_value(run(&book)).unwrap();
        assert_eq!(v["surplus"], json!("2000000"));
        assert_eq!(v["fills"][0]["side"], json!("sell"));
    }
}
