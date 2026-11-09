//! Reconciliation against the verifier between batches: fills, cancel
//! tickets, settlements that never happened, balances that cannot pay.

mod common;

use blindbook_matcher::batch::{SETTLE_GUARD_NS, VENUE_TTL_S};
use blindbook_matcher::util::NS_PER_S;
use common::*;
use serde_json::{json, Value};

struct Pair {
    h: Harness,
    seller: Trader,
    buyer: Trader,
}

fn pair() -> Pair {
    let mut h = Harness::default();
    let seller = Trader::named("seller.testnet", 1);
    let buyer = Trader::implicit(2);
    h.fund_both(&seller);
    h.fund_both(&buyer);
    Pair { h, seller, buyer }
}

fn resting_lots(book: &Value) -> usize {
    book["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["lots"].as_array().unwrap().len())
        .sum()
}

fn in_flight(book: &Value) -> usize {
    book["in_flight"].as_array().unwrap().len()
}

/// Move both the matcher's and the verifier's clocks.
fn advance(h: &mut Harness, ns: u64) {
    h.host.now += ns;
    h.host.verifier.now_ns += ns;
    // the 1Click fixtures are fixed in time
    h.params["quote_max_age_s"] = json!(0);
}

#[test]
fn a_cancel_ticket_drops_the_resting_lot() {
    let Pair { mut h, seller, .. } = pair();
    let o = h.order(&seller, "sell", &[SELL_LIMIT, SELL_LIMIT, SELL_LIMIT], 1, 1);
    h.run(&[&o]);
    assert_eq!(resting_lots(&h.book()), 3);
    // the trader consumes one lot's nonce with an empty-intents payload
    h.host
        .verifier
        .used
        .insert((seller.account.clone(), o.nonces()[1].clone()));
    let r = h.run(&[]).result;
    assert_eq!(r["book"], json!({ "orders": 1, "lots": 2 }));
    let left: Vec<u64> = h.book()["orders"][0]["lots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["index"].as_u64().unwrap())
        .collect();
    assert_eq!(left, [0, 2]);
    // cancelling the rest removes the order
    for n in o.nonces() {
        h.host.verifier.used.insert((seller.account.clone(), n));
    }
    let r = h.run(&[]).result;
    assert_eq!(r["book"], json!({ "orders": 0, "lots": 0 }));
}

#[test]
fn an_order_left_below_its_minimum_is_dropped() {
    let Pair { mut h, seller, .. } = pair();
    let o = h.order(&seller, "sell", &[SELL_LIMIT; 3], 3, 1);
    h.run(&[&o]);
    h.host
        .verifier
        .used
        .insert((seller.account.clone(), o.nonces()[0].clone()));
    let r = h.run(&[]).result;
    assert_eq!(
        r["book"],
        json!({ "orders": 0, "lots": 0 }),
        "2 lots left can never fill a 3-lot minimum"
    );
}

#[test]
fn a_used_nonce_on_an_in_flight_lot_is_a_fill() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT; 2], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    let r = h.run(&[&s, &b]).result;
    assert_eq!(in_flight(&h.book()), 2);
    h.settle(&r);
    let r2 = h.run(&[]).result;
    let book = h.book();
    assert_eq!(in_flight(&book), 0);
    assert_eq!(resting_lots(&book), 1, "the seller's second lot still rests");
    assert_eq!(r2["book"], json!({ "orders": 1, "lots": 1 }));
}

#[test]
fn an_unsettled_lot_stays_in_flight_until_its_sub_batch_expires() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    h.run(&[&s, &b]);
    // the pool never executed the sub-batch (or it failed): nonces unused
    let r = h.run(&[]).result;
    assert_eq!(in_flight(&h.book()), 2, "the venue payload may still settle");
    assert!(
        r["settle"].as_array().unwrap().is_empty(),
        "in-flight lots never cross twice"
    );

    advance(&mut h, VENUE_TTL_S * NS_PER_S);
    h.run(&[]);
    assert_eq!(in_flight(&h.book()), 2, "inside the guard window after the deadline");

    advance(&mut h, SETTLE_GUARD_NS + NS_PER_S);
    let r = h.run(&[]).result;
    // both lots are back, and since they still cross they go out again
    assert_eq!(r["batches"][0]["matched_lots"], 1);
    assert_eq!(r["settle"].as_array().unwrap().len(), 1);
    let book = h.book();
    assert_eq!(in_flight(&book), 2);
    let settle_by = book["in_flight"][0]["settle_by_ns"].as_u64().unwrap();
    assert_eq!(settle_by, (h.host.now / NS_PER_S + VENUE_TTL_S) * NS_PER_S);
}

#[test]
fn an_unsettled_lot_returns_to_the_book_with_its_order() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT, SELL_LIMIT], 2, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT, BUY_LIMIT], 1, 9);
    h.run(&[&s, &b]);
    assert_eq!(
        h.book()["orders"].as_array().unwrap().len(),
        0,
        "both orders fully in flight"
    );
    // the buyer's funds are gone, so after the return nothing crosses again
    h.host
        .verifier
        .balances
        .insert((buyer.account.clone(), QUOTE.into()), 0);
    advance(&mut h, VENUE_TTL_S * NS_PER_S + SETTLE_GUARD_NS + NS_PER_S);
    let r = h.run(&[]).result;
    let book = h.book();
    assert_eq!(in_flight(&book), 0);
    assert_eq!(resting_lots(&book), 4);
    let seller_order = book["orders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["meta"]["id"] == s.id)
        .unwrap();
    assert_eq!(seller_order["meta"]["min_lots"], 2, "metadata survives the round trip");
    assert_eq!(r["book"], json!({ "orders": 2, "lots": 4 }));
    assert!(r["settle"].as_array().unwrap().is_empty());
}

#[test]
fn an_unsettled_lot_past_its_own_deadline_is_dropped() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    h.run(&[&s, &b]);
    advance(&mut h, (DEADLINE_S - NOW_S + 1) * NS_PER_S);
    let r = h.run(&[]).result;
    assert_eq!(r["book"], json!({ "orders": 0, "lots": 0 }));
}

#[test]
fn an_unknown_nonce_keeps_lots_where_they_are() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    h.run(&[&s, &b]);
    h.run(&[]); // nothing settled; lots still in flight
    advance(&mut h, VENUE_TTL_S * NS_PER_S + SETTLE_GUARD_NS + NS_PER_S);
    h.host.verifier.down.insert("is_nonce_used".into());
    let r = h.run(&[]).result;
    assert_eq!(in_flight(&h.book()), 2, "no answer, no decision");
    assert!(r["settle"].as_array().unwrap().is_empty());
}

#[test]
fn resting_lots_with_unknown_status_do_not_cross() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    h.run(&[&s]);
    h.host.verifier.down.insert("is_valid_salt".into());
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    let r = h.run(&[&b]).result;
    assert_eq!(ids(&r["deferred"]), std::slice::from_ref(&b.id));
    assert!(r["settle"].as_array().unwrap().is_empty());
    assert_eq!(resting_lots(&h.book()), 1, "kept, just not crossed");
}

#[test]
fn an_invalidated_salt_drops_resting_lots() {
    let Pair { mut h, seller, .. } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    h.run(&[&s]);
    h.host.verifier.valid_salts.clear();
    h.host.verifier.valid_salts.insert("aabbccdd".into());
    h.host.verifier.salt = "aabbccdd".into();
    let r = h.run(&[]).result;
    assert_eq!(r["book"], json!({ "orders": 0, "lots": 0 }));
}

#[test]
fn an_owner_who_cannot_pay_is_excluded_but_kept() {
    let mut h = Harness::default();
    let seller = Trader::named("seller.testnet", 1);
    let buyer = Trader::implicit(2);
    h.fund_both(&buyer);
    // one lot of base for a two-lot sell order
    h.fund(&seller, BASE, LOT);
    let s = h.order(&seller, "sell", &[SELL_LIMIT, SELL_LIMIT], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT, BUY_LIMIT], 1, 9);
    let r = h.run(&[&s, &b]).result;
    assert_eq!(ids(&r["ingested"]), ["1", "2"]);
    assert_eq!(r["batches"][0]["skipped"], "no_cross");
    assert!(r["settle"].as_array().unwrap().is_empty());
    assert_eq!(
        r["book"],
        json!({ "orders": 2, "lots": 4 }),
        "excluded from the round, not from the book"
    );

    h.fund(&seller, BASE, LOT);
    let r = h.run(&[]).result;
    assert_eq!(r["batches"][0]["matched_lots"], 2);
}

#[test]
fn a_buyer_needs_the_sum_of_their_limits() {
    let mut h = Harness::default();
    let seller = Trader::named("seller.testnet", 1);
    let buyer = Trader::evm(3);
    h.fund_both(&seller);
    h.fund(&buyer, QUOTE, 2 * BUY_LIMIT - 1);
    let s = h.order(&seller, "sell", &[SELL_LIMIT; 2], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT; 2], 1, 9);
    let r = h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["skipped"], "no_cross");
    h.fund(&buyer, QUOTE, 1);
    let r = h.run(&[]).result;
    assert_eq!(r["batches"][0]["matched_lots"], 2);
}

#[test]
fn one_balance_backs_an_owners_orders_in_time_priority() {
    let mut h = Harness::default();
    let seller = Trader::named("seller.testnet", 1);
    let buyer = Trader::implicit(2);
    h.fund_both(&buyer);
    h.fund(&seller, BASE, LOT);
    // two one-lot sells, enough base for one: only the earlier is eligible
    let first = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    let second = h.order(&seller, "sell", &[SELL_LIMIT - 10_000], 1, 2);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT; 2], 1, 9);
    let r = h.run(&[&first, &second, &b]).result;
    assert_eq!(r["batches"][0]["matched_lots"], 1);
    let flying = &h.book()["in_flight"];
    assert!(flying.as_array().unwrap().iter().any(|f| f["meta"]["id"] == first.id));
    assert!(!flying.as_array().unwrap().iter().any(|f| f["meta"]["id"] == second.id));
}

#[test]
fn an_unknown_balance_excludes_the_order() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    h.host.verifier.down.insert("mt_balance_of".into());
    let r = h.run(&[&s, &b]).result;
    assert_eq!(ids(&r["ingested"]), ["1", "2"]);
    assert!(r["settle"].as_array().unwrap().is_empty());
}

#[test]
fn expired_resting_lots_are_dropped() {
    let Pair { mut h, seller, .. } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    h.run(&[&s]);
    advance(&mut h, (DEADLINE_S - NOW_S) * NS_PER_S);
    let r = h.run(&[]).result;
    assert_eq!(r["book"], json!({ "orders": 0, "lots": 0 }));
}

#[test]
fn lots_inside_the_deadline_margin_rest_but_do_not_cross() {
    let Pair { mut h, seller, buyer } = pair();
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    h.run(&[&s]);
    // 60 s before the lots expire, inside the 90 s margin
    advance(&mut h, (DEADLINE_S - NOW_S - 60) * NS_PER_S);
    let late = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    let r = h.run(&[&late]).result;
    assert_eq!(rejected_reason(&r, &late.id).as_deref(), Some("expired"));
    assert!(r["settle"].as_array().unwrap().is_empty());
    assert_eq!(r["book"]["lots"], 1);
}
