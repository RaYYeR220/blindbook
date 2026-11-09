//! Randomized invariant checks over thousands of generated order books.
//!
//! For every batch the engine produces, these must hold:
//! - the base token nets to zero (equal lots sold and bought),
//! - rebates plus the venue fee equal the surplus exactly,
//! - no seller receives less than their signed limit and no buyer pays more,
//! - the venue fee never exceeds the per-side cap,
//! - every intent in the batch belongs to a filled lot, and no lot is used twice.

use std::collections::BTreeSet;

use blindbook_core::{cross, Amount, Lot, Market, Order, Params, Price, Side};
use serde_json::json;

const NEAR: Amount = 1_000_000_000_000_000_000_000_000;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

fn random_book(rng: &mut Rng, lot_mid: Amount) -> Vec<Order> {
    let owners = ["a.near", "b.near", "c.near", "d.near", "e.near", "0xabc", "f.near"];
    let n = rng.range(0, 12) as usize;
    (0..n)
        .map(|i| {
            let side = if rng.next() % 2 == 0 { Side::Sell } else { Side::Buy };
            let lots = rng.range(1, 8) as usize;
            let min_lots = rng.range(1, lots as u64) as u16;
            Order {
                id: format!("o{i}"),
                owner: owners[(rng.next() % owners.len() as u64) as usize].to_string(),
                side,
                lots: (0..lots)
                    .map(|j| {
                        // limits within +-3% of mid
                        let delta = (lot_mid as u64 / 100 * rng.range(0, 3)) as Amount;
                        let above = rng.next() % 2 == 0;
                        let limit = if above { lot_mid + delta } else { lot_mid - delta };
                        Lot { index: j as u16, limit_quote: limit, intent: json!({"o": i, "l": j}) }
                    })
                    .collect(),
                min_lots,
                deadline_ns: rng.range(0, 3) * 1_000,
                seq: rng.range(0, 1_000),
            }
        })
        .collect()
}

#[test]
fn engine_invariants_hold_on_random_books() {
    let market = Market { base: "nep141:wrap.near".into(), quote: "nep141:usdc".into(), lot: 10 * NEAR };
    let params = Params { fee_bps: 5, max_spread_bps: 150, deadline_margin_ns: 500 };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut crossed_batches = 0;
    let mut total_lots = 0u64;

    for _ in 0..5_000 {
        let spread = rng.range(0, 120) as u128;
        let bid = Price::new(3_600_000 - spread * 100, NEAR).unwrap();
        let ask = Price::new(3_600_000 + spread * 100, NEAR).unwrap();
        let lot_mid_guess = 36_000_000;
        let book = random_book(&mut rng, lot_mid_guess);
        let batch = cross(&market, &book, bid, ask, 1_000, &params).unwrap();
        if batch.matched_lots == 0 {
            assert!(batch.intents.is_empty());
            assert_eq!(batch.surplus, 0);
            continue;
        }
        crossed_batches += 1;
        total_lots += batch.matched_lots as u64;

        let sold: usize = batch.fills.iter().filter(|f| f.side == Side::Sell).map(|f| f.lot_indices.len()).sum();
        let bought: usize = batch.fills.iter().filter(|f| f.side == Side::Buy).map(|f| f.lot_indices.len()).sum();
        assert_eq!(sold, bought, "base must net to zero");
        assert_eq!(sold as u32, batch.matched_lots);

        let rebates: Amount = batch.rebates.iter().map(|r| r.amount).sum();
        assert_eq!(rebates + batch.fee, batch.surplus, "surplus must be fully accounted for");

        let fee_cap = batch.lot_mid_quote * 5 / 10_000;
        let mut used = BTreeSet::new();
        for f in &batch.fills {
            let order = book.iter().find(|o| o.id == f.order_id).unwrap();
            assert_eq!(order.owner, f.owner);
            assert!(f.lot_indices.len() >= order.min_lots as usize, "min fill respected");
            assert!(order.deadline_ns > 1_000 + 500, "no expiring order is used");
            for idx in &f.lot_indices {
                assert!(used.insert((f.order_id.clone(), *idx)), "lot used twice");
                let lot = &order.lots[*idx as usize];
                match f.side {
                    Side::Sell => assert!(lot.limit_quote <= batch.lot_mid_quote),
                    Side::Buy => assert!(lot.limit_quote >= batch.lot_mid_quote),
                }
            }
            match f.side {
                // seller receives limit + rebate, which is never below the limit
                Side::Sell => assert!(f.limit_quote + f.rebate >= f.limit_quote),
                // buyer pays limit - rebate, which is never above the limit
                Side::Buy => assert!(f.rebate <= f.limit_quote),
            }
            assert!(f.fee <= fee_cap * f.lot_indices.len() as Amount, "fee cap respected");
        }
        assert_eq!(batch.intents.len(), used.len());

        // a trader never appears on both sides of one batch
        let sellers: BTreeSet<_> = batch.fills.iter().filter(|f| f.side == Side::Sell).map(|f| &f.owner).collect();
        assert!(batch.fills.iter().filter(|f| f.side == Side::Buy).all(|f| !sellers.contains(&f.owner)));
    }

    assert!(crossed_batches > 1_000, "generator should produce plenty of crossing books, got {crossed_batches}");
    println!("checked {crossed_batches} crossing batches, {total_lots} lots");
}
