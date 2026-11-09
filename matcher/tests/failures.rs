//! Failure handling: no price is ever made up, a refused sub-batch never
//! settles, and an internal failure fails the whole run closed.

mod common;

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

fn crossing(p: &mut Pair) -> (Order, Order) {
    let s = p.h.order(&p.seller, "sell", &[SELL_LIMIT], 1, 1);
    let b = p.h.order(&p.buyer, "buy", &[BUY_LIMIT], 1, 9);
    (s, b)
}

fn resting(book: &Value) -> usize {
    book["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["lots"].as_array().unwrap().len())
        .sum()
}

fn assert_no_settlement(r: &Value) {
    assert!(r["settle"].as_array().unwrap().is_empty());
    assert_eq!(r["batches"][0]["matched_lots"], 0);
}

// ---------------------------------------------------------------- quotes

#[test]
fn a_failed_quote_skips_the_market() {
    for (mode, skipped) in [
        (QuoteMode::Status(500), "quote_unavailable"),
        (QuoteMode::Status(429), "quote_unavailable"),
        (QuoteMode::Down, "quote_unavailable"),
        (QuoteMode::Tampered, "quote_unverified"),
    ] {
        let mut p = pair();
        let (s, b) = crossing(&mut p);
        p.h.host.quotes = mode;
        let r = p.h.run(&[&s, &b]).result;
        assert_eq!(r["batches"][0]["skipped"], skipped, "{mode:?}");
        assert_eq!(r["batches"][0]["mid"], Value::Null, "no price is made up");
        assert_no_settlement(&r);
        assert_eq!(ids(&r["ingested"]), ["1", "2"], "orders are still ingested");
        assert_eq!(resting(&p.h.book()), 2);
    }
}

#[test]
fn a_stale_quote_skips_the_market_unless_the_window_is_off() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.host.now += 600 * 1_000_000_000;
    p.h.host.verifier.now_ns += 600 * 1_000_000_000;
    let r = p.h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["skipped"], "quote_stale");
    assert_no_settlement(&r);
    p.h.params["quote_max_age_s"] = json!(0);
    let r = p.h.run(&[]).result;
    assert_eq!(r["batches"][0]["matched_lots"], 1);
}

#[test]
fn a_wide_reference_spread_skips_the_market() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    // the fixture spread is ~44 bps
    p.h.params["max_spread_bps"] = json!(10);
    let r = p.h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["skipped"], "bad_reference");
    assert_no_settlement(&r);
}

#[test]
fn fee_or_salt_unavailable_skips_settlement_but_not_ingestion() {
    for view in ["fee", "current_salt"] {
        let mut p = pair();
        let (s, b) = crossing(&mut p);
        p.h.host.verifier.down.insert(view.into());
        let r = p.h.run(&[&s, &b]).result;
        assert_eq!(r["batches"][0]["skipped"], "rpc_unavailable", "{view}");
        assert_eq!(ids(&r["ingested"]), ["1", "2"]);
        assert_no_settlement(&r);
    }
}

// ---------------------------------------------------------------- simulation

#[test]
fn a_panicking_simulation_drops_the_sub_batch_and_keeps_its_lots() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.host.verifier.sim = SimMode::Panic;
    let r = p.h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["dropped"], json!(["simulate_failed"]));
    assert_eq!(r["batches"][0]["sub_batches"], 0);
    assert_no_settlement(&r);
    let book = p.h.book();
    assert_eq!(resting(&book), 2, "lots stay in the book");
    assert!(
        book["in_flight"].as_array().unwrap().is_empty(),
        "nothing is marked in flight"
    );
}

#[test]
fn an_invariant_violation_drops_the_sub_batch_without_blaming_traders() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.host.verifier.sim = SimMode::Invariant;
    for _ in 0..4 {
        let r = p.h.run(&[&s, &b]).result;
        assert_eq!(r["batches"][0]["dropped"], json!(["simulate_failed"]));
        assert_no_settlement(&r);
    }
    assert_eq!(
        p.h.host.count("simulate_intents"),
        1,
        "no per-lot isolation for a closure mismatch"
    );
    let book = p.h.book();
    assert_eq!(resting(&book), 2);
    assert!(book["orders"].as_array().unwrap().iter().all(|o| o["strikes"] == 0));
}

#[test]
fn an_unreachable_simulation_drops_the_sub_batch() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.host.verifier.down.insert("simulate_intents".into());
    let r = p.h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["dropped"], json!(["simulate_unavailable"]));
    assert_no_settlement(&r);
    assert_eq!(resting(&p.h.book()), 2);
}

#[test]
fn an_order_that_keeps_breaking_sub_batches_is_struck_out() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    // the buyer revoked its key after ingestion: every sub-batch with it panics
    p.h.host.verifier.poisoned.insert(p.buyer.account.clone());
    for round in 1..=3 {
        let r = p.h.run(&[&s, &b]).result;
        assert_eq!(r["batches"][0]["dropped"], json!(["simulate_failed"]), "round {round}");
        // the batch plus one solo simulation per lot
        assert_eq!(p.h.host.count("simulate_intents"), 3);
    }
    let book = p.h.book();
    let ids: Vec<&str> = book["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["meta"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [s.id.as_str()],
        "the buyer is gone after three strikes, the seller untouched"
    );
    assert_eq!(book["orders"][0]["strikes"], 0);

    // an honest buyer now crosses with the seller
    let honest = Trader::evm(5);
    p.h.fund_both(&honest);
    let b2 = p.h.order(&honest, "buy", &[BUY_LIMIT], 1, 33);
    let r = p.h.run(&[&b2]).result;
    assert_eq!(r["batches"][0]["matched_lots"], 1);
    p.h.settle(&r);
}

#[test]
fn a_venue_without_dust_drops_sub_batches_it_cannot_pay_for() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.host.verifier.balances.insert((VENUE.into(), BASE.into()), 0);
    let r = p.h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["dropped"], json!(["venue_inventory"]));
    assert_no_settlement(&r);
    assert_eq!(p.h.host.count("simulate_intents"), 0);
    assert_eq!(resting(&p.h.book()), 2);
}

#[test]
fn venue_dust_is_budgeted_across_sub_batches() {
    let mut p = pair();
    p.h.params["max_lots_per_side"] = json!(1);
    let s = p.h.order(&p.seller, "sell", &[SELL_LIMIT; 3], 1, 1);
    let b = p.h.order(&p.buyer, "buy", &[BUY_LIMIT; 3], 1, 9);
    // first find what one sub-batch costs the venue in base
    let mut probe = pair();
    probe.h.params["max_lots_per_side"] = json!(1);
    let ps = probe.h.order(&probe.seller, "sell", &[SELL_LIMIT], 1, 1);
    let pb = probe.h.order(&probe.buyer, "buy", &[BUY_LIMIT], 1, 9);
    let pr = probe.h.run(&[&ps, &pb]).result;
    let venue = &pr["settle"][0][2];
    let msg: Value = serde_json::from_str(venue["payload"]["message"].as_str().unwrap()).unwrap();
    let dust: i128 = msg["intents"][0]["diff"][BASE].as_str().unwrap().parse().unwrap();
    assert!(dust < 0, "the venue pays the protocol fee on base");
    // enough for exactly two sub-batches
    p.h.host
        .verifier
        .balances
        .insert((VENUE.into(), BASE.into()), 2 * dust.unsigned_abs());
    let r = p.h.run(&[&s, &b]).result;
    assert_eq!(r["batches"][0]["sub_batches"], 2);
    assert_eq!(r["batches"][0]["dropped"], json!(["venue_inventory"]));
    p.h.settle(&r);
}

// ---------------------------------------------------------------- fail closed

fn assert_failed_closed(run: &Run, code: &str) {
    let r = &run.result;
    assert_eq!(r["error"], code);
    assert_eq!(r["op"], "batch");
    for k in ["ingested", "rejected", "deferred", "batches", "settle"] {
        assert!(r[k].as_array().unwrap().is_empty(), "{k} must be empty");
    }
    assert_eq!(r["book"], Value::Null);
}

#[test]
fn a_book_changed_by_another_run_fails_closed() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.run(&[&s]);
    let before = p.h.host.storage.clone();
    p.h.host.cas_conflict = true;
    let run = p.h.run(&[&b]);
    assert_failed_closed(&run, "book_conflict");
    assert_eq!(p.h.host.storage, before, "the stored book is untouched");
}

#[test]
fn storage_failures_fail_closed() {
    let mut p = pair();
    let (s, _) = crossing(&mut p);
    p.h.host.storage_down = true;
    let run = p.h.run(&[&s]);
    assert_failed_closed(&run, "storage_unavailable");
    assert!(p.h.host.storage.is_empty());
}

#[test]
fn a_corrupt_book_fails_closed_and_is_kept_for_inspection() {
    let mut p = pair();
    let (s, _) = crossing(&mut p);
    p.h.host.storage.insert("book".into(), b"{\"v\":9}".to_vec());
    let run = p.h.run(&[&s]);
    assert_failed_closed(&run, "book_corrupt");
    assert_eq!(p.h.host.storage["book"], b"{\"v\":9}");
    p.h.host.storage.insert("book".into(), b"not json".to_vec());
    assert_failed_closed(&p.h.run(&[&s]), "book_corrupt");
}

#[test]
fn without_keys_nothing_is_signed_or_stored() {
    let mut p = pair();
    let (s, b) = crossing(&mut p);
    p.h.host.keys_down = true;
    let input = p.h.input(&[s.input(), b.input()]).to_string();
    let out = blindbook_matcher::run(&mut p.h.host, input.as_bytes());
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v, json!({ "error": "result_key_unavailable" }));
    assert!(p.h.host.storage.is_empty());
}

#[test]
fn bad_input_fails_closed() {
    let mut p = pair();
    let (s, _) = crossing(&mut p);
    let good = p.h.input(&[s.input()]);
    let cases: Vec<(Value, &str)> = vec![
        (
            {
                let mut v = good.clone();
                v["v"] = json!(2);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["network"] = json!("mainnet");
                v
            },
            "network_mismatch",
        ),
        (
            {
                let mut v = good.clone();
                v["params"]["max_lots_per_side"] = json!(7);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["params"]["max_sub_batches"] = json!(0);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["params"]["fee_bps"] = json!(5_000);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["venue_account"] = json!("Not Valid");
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v.as_object_mut().unwrap().remove("venue_account");
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["now_ns"] = json!(12);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["markets"][0]["lot"] = json!("0");
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["markets"][0]["quote"] = json!(BASE);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                let m = v["markets"][0].clone();
                v["markets"].as_array_mut().unwrap().push(m);
                v
            },
            "bad_input",
        ),
        (
            {
                let mut v = good.clone();
                v["orders"] = json!(vec![s.input(); 65]);
                v
            },
            "bad_input",
        ),
    ];
    for (i, (input, code)) in cases.into_iter().enumerate() {
        let run = p.h.run_raw(input.to_string().as_bytes());
        assert_failed_closed(&run, code);
        assert!(p.h.host.storage.is_empty(), "case {i}");
        assert!(
            p.h.host.calls.is_empty(),
            "case {i}: nothing is fetched for a bad input"
        );
    }
}

#[test]
fn unparseable_input_is_answered_with_a_signed_error() {
    let mut h = Harness::default();
    for input in [&b"not json"[..], b"{}", br#"{"v":1,"op":"steal"}"#] {
        let run = h.run_raw(input);
        assert_eq!(run.result["error"], "bad_input");
        assert_eq!(run.result["op"], "unknown");
    }
    let run = h.run_raw(br#"{"v":2,"op":"keys"}"#);
    assert_eq!(run.result, json!({ "v": 1, "op": "keys", "error": "bad_input" }));
}

#[test]
fn stderr_is_never_needed_errors_are_codes() {
    // every failure path above answers on stdout with a short code; here we
    // check the codes carry no detail
    let mut p = pair();
    let (mut s, _) = crossing(&mut p);
    s.sealed = "AAAA".into();
    let run = p.h.run(&[&s]);
    let r: Value = run.result;
    assert_eq!(r["rejected"][0], json!({ "id": s.id, "reason": "invalid" }));
}
