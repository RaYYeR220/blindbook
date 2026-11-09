//! Bounds: book size, lots per order, sub-batches across markets.

mod common;

use blindbook_matcher::book::{MAX_LOTS, MAX_ORDERS};
use blindbook_matcher::ingest::MAX_LOTS_PER_ORDER;
use common::*;
use serde_json::json;

#[test]
fn the_book_holds_at_most_64_orders() {
    let mut h = Harness::default();
    let traders: Vec<Trader> = (0..MAX_ORDERS + 1).map(|i| Trader::implicit(100 + i as u8)).collect();
    for t in &traders {
        h.fund_both(t);
    }
    let orders: Vec<Order> = traders[..MAX_ORDERS]
        .iter()
        .map(|t| h.order(t, "sell", &[SELL_LIMIT], 1, 1))
        .collect();
    let refs: Vec<&Order> = orders.iter().collect();
    let r = h.run(&refs).result;
    assert_eq!(r["ingested"].as_array().unwrap().len(), MAX_ORDERS);
    assert_eq!(r["book"], json!({ "orders": MAX_ORDERS, "lots": MAX_ORDERS }));

    let extra = h.order(&traders[MAX_ORDERS], "sell", &[SELL_LIMIT], 1, 1);
    let r = h.run(&[&extra]).result;
    assert_eq!(
        ids(&r["deferred"]),
        std::slice::from_ref(&extra.id),
        "full book: the order waits at the pool"
    );
    assert!(r["rejected"].as_array().unwrap().is_empty());
    assert_eq!(r["book"]["orders"], MAX_ORDERS);

    // a cancel frees a slot and the waiting order gets in
    h.host
        .verifier
        .used
        .insert((traders[0].account.clone(), orders[0].nonces()[0].clone()));
    let r = h.run(&[&extra]).result;
    assert_eq!(ids(&r["ingested"]), std::slice::from_ref(&extra.id));
    assert_eq!(r["book"]["orders"], MAX_ORDERS);
}

#[test]
fn the_book_holds_at_most_512_lots() {
    let mut h = Harness::default();
    let per = MAX_LOTS_PER_ORDER;
    let n = MAX_LOTS / per;
    let traders: Vec<Trader> = (0..n + 1).map(|i| Trader::evm(40 + i as u8)).collect();
    for t in &traders {
        h.fund_both(t);
    }
    let orders: Vec<Order> = traders[..n]
        .iter()
        .map(|t| h.order(t, "sell", &vec![SELL_LIMIT; per], 1, 1))
        .collect();
    let refs: Vec<&Order> = orders.iter().collect();
    let r = h.run(&refs).result;
    assert_eq!(r["book"], json!({ "orders": n, "lots": MAX_LOTS }));

    let one_more = h.order(&traders[n], "sell", &[SELL_LIMIT], 1, 1);
    let r = h.run(&[&one_more]).result;
    assert_eq!(ids(&r["deferred"]), std::slice::from_ref(&one_more.id));
    assert_eq!(r["book"]["lots"], MAX_LOTS);
    // the stored book stays well under the size cap
    assert!(h.host.storage["book"].len() < blindbook_matcher::book::MAX_BOOK_BYTES);
}

#[test]
fn an_order_may_carry_at_most_64_lots() {
    let mut h = Harness::default();
    let t = Trader::implicit(9);
    h.fund_both(&t);
    let ok = h.order(&t, "sell", &vec![SELL_LIMIT; MAX_LOTS_PER_ORDER], 1, 1);
    let too_big = h.order(&t, "sell", &vec![SELL_LIMIT; MAX_LOTS_PER_ORDER + 1], 1, 100);
    let r = h.run(&[&ok, &too_big]).result;
    assert_eq!(ids(&r["ingested"]), std::slice::from_ref(&ok.id));
    assert_eq!(rejected_reason(&r, &too_big.id).as_deref(), Some("invalid"));
}

#[test]
fn the_sub_batch_cap_is_shared_by_all_markets() {
    let mut h = Harness::default();
    h.params["max_sub_batches"] = json!(1);
    let seller = Trader::named("seller.testnet", 1);
    let buyer = Trader::implicit(2);
    h.fund_both(&seller);
    h.fund_both(&buyer);
    let s = h.order(&seller, "sell", &[SELL_LIMIT], 1, 1);
    let b = h.order(&buyer, "buy", &[BUY_LIMIT], 1, 9);
    // a second market with the same reference but other test tokens
    let mut input = h.input(&[s.input(), b.input()]);
    let mut m2 = input["markets"][0].clone();
    m2["id"] = json!("other");
    m2["base"] = json!("nep141:tnear2.testnet");
    m2["quote"] = json!("nep141:tusdc2.testnet");
    input["markets"].as_array_mut().unwrap().push(m2);
    let r = h.run_raw(input.to_string().as_bytes()).result;
    assert_eq!(r["batches"].as_array().unwrap().len(), 2);
    assert_eq!(r["batches"][0]["sub_batches"], 1);
    assert_eq!(r["batches"][1]["skipped"], "no_cross");
    assert_eq!(r["settle"].as_array().unwrap().len(), 1);
    assert_eq!(h.host.count("quote"), 4, "every market is priced every batch");
}

#[test]
fn markets_are_priced_even_with_an_empty_book() {
    let mut h = Harness::default();
    let r = h.run(&[]).result;
    assert_eq!(r["batches"][0]["skipped"], "no_cross");
    assert!(r["batches"][0]["mid"].is_object());
    assert_eq!(h.host.count("quote"), 2);
    assert_eq!(r["book"], json!({ "orders": 0, "lots": 0 }));
}
