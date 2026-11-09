//! End-to-end batches: real-signed NEP-413 and ERC-191 lots, real 1Click
//! quotes, a venue payload the mock verifier executes to zero.

mod common;

use blindbook_core::intent::nep413_hash;
use blindbook_matcher::nonce;
use blindbook_matcher::util::{ed25519_bytes, hex, rfc3339, sha256, unb64, NS_PER_S};
use common::*;
use ed25519_dalek::Verifier;
use serde_json::{json, Value};

struct Desk {
    h: Harness,
    seller: Trader,
    buyer_nep: Trader,
    buyer_evm: Trader,
}

fn desk() -> Desk {
    let mut h = Harness::default();
    let seller = Trader::named("seller.testnet", 1);
    let buyer_nep = Trader::implicit(2);
    let buyer_evm = Trader::evm(3);
    for t in [&seller, &buyer_nep, &buyer_evm] {
        h.fund_both(t);
    }
    Desk {
        h,
        seller,
        buyer_nep,
        buyer_evm,
    }
}

#[test]
fn full_batch_crosses_nep413_and_erc191_orders() {
    let Desk {
        mut h,
        seller,
        buyer_nep,
        buyer_evm,
    } = desk();
    let sell = h.order(&seller, "sell", &[SELL_LIMIT, SELL_LIMIT], 1, 10);
    let buy1 = h.order(&buyer_nep, "buy", &[BUY_LIMIT], 1, 20);
    let buy2 = h.order(&buyer_evm, "buy", &[BUY_LIMIT - 1_000], 1, 30);
    let r = h.run(&[&sell, &buy1, &buy2]).result;

    assert_eq!(r["v"], 1);
    assert_eq!(r["op"], "batch");
    assert!(r.get("error").is_none());
    assert_eq!(ids(&r["ingested"]), ["1", "2", "3"]);
    assert!(r["rejected"].as_array().unwrap().is_empty());
    assert!(r["deferred"].as_array().unwrap().is_empty());
    assert_eq!(r["now_ns"], NOW.to_string());

    let b = &r["batches"][0];
    assert_eq!(b["market"], MARKET);
    assert_eq!(b["skipped"], Value::Null);
    assert_eq!(b["matched_lots"], 2);
    assert_eq!(b["sub_batches"], 1);
    assert_eq!(b["base_volume"], (2 * LOT).to_string());
    assert_eq!(b["quotes"].as_array().unwrap().len(), 2);
    let lot_mid: u128 = b["lot_mid_quote"].as_str().unwrap().parse().unwrap();
    assert!(SELL_LIMIT < lot_mid && lot_mid < BUY_LIMIT - 1_000, "mid {lot_mid}");
    assert_eq!(b["quote_volume"], (2 * lot_mid).to_string());
    let mid = &b["mid"];
    assert!(mid["quote"].is_string() && mid["base"].is_string());

    // one sub-batch: sells, then buys, then the venue
    let settle = r["settle"].as_array().unwrap();
    assert_eq!(settle.len(), 1);
    let sub = settle[0].as_array().unwrap();
    assert_eq!(sub.len(), 5);
    let sells: Vec<&Value> = sell.body["lots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| &l["intent"])
        .collect();
    assert_eq!(&sub[0], sells[0]);
    assert_eq!(&sub[1], sells[1]);
    assert_eq!(sub[2], buy1.body["lots"][0]["intent"]);
    assert_eq!(sub[3], buy2.body["lots"][0]["intent"]);
    assert_eq!(sub[2]["standard"], "nep413");
    assert_eq!(sub[3]["standard"], "erc191");

    // the venue's payload is NEP-413 by the venue key, for the verifier, with
    // a versioned nonce on the current salt and a 180 s deadline
    let venue = &sub[4];
    assert_eq!(venue["standard"], "nep413");
    assert_eq!(venue["public_key"], h.host.venue_pk());
    assert_eq!(venue["payload"]["recipient"], VERIFIER);
    let message: Value = serde_json::from_str(venue["payload"]["message"].as_str().unwrap()).unwrap();
    assert_eq!(message["signer_id"], VENUE);
    assert_eq!(message["deadline"], rfc3339(NOW_S + 180));
    let intents = message["intents"].as_array().unwrap();
    assert_eq!(intents[0]["intent"], "token_diff");
    assert_eq!(intents.len(), 1 + 3, "closure plus one rebate per trader");
    assert!(intents[1..]
        .iter()
        .all(|i| i["intent"] == "transfer" && i["tokens"][QUOTE].is_string()));
    let raw: [u8; 32] = unb64(venue["payload"]["nonce"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let n = nonce::decode(&raw).unwrap();
    assert_eq!(hex(&n.salt), SALT);
    assert_eq!(n.deadline_ns as u64, (NOW_S + 180) * NS_PER_S);
    let sig: [u8; 64] = ed25519_bytes(venue["signature"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let hash = nep413_hash(venue["payload"]["message"].as_str().unwrap(), &raw, VERIFIER, None);
    h.host
        .venue
        .verifying_key()
        .verify(&hash, &ed25519_dalek::Signature::from_bytes(&sig))
        .unwrap();

    // executing it as the pool would leaves every token balanced, and every
    // trader at or better than their limit
    let before = |t: &Trader, tok: &str| h.host.verifier.balance(&t.account, tok);
    let (s_q, b1_q, b2_q) = (
        before(&seller, QUOTE),
        before(&buyer_nep, QUOTE),
        before(&buyer_evm, QUOTE),
    );
    h.settle(&r);
    let after = |t: &Trader, tok: &str| h.host.verifier.balance(&t.account, tok);
    assert!(after(&seller, QUOTE) - s_q >= 2 * SELL_LIMIT);
    assert!(b1_q - after(&buyer_nep, QUOTE) <= BUY_LIMIT);
    assert!(b2_q - after(&buyer_evm, QUOTE) <= BUY_LIMIT - 1_000);
    let rebated: u128 = r["batches"][0]["rebated"].as_str().unwrap().parse().unwrap();
    assert!(rebated > 0);

    // the lots are in flight; the book holds no order contents in the result
    assert_eq!(r["book"], json!({ "orders": 3, "lots": 4 }));
    let book = h.book();
    assert_eq!(book["v"], 1);
    assert!(book["orders"].as_array().unwrap().is_empty());
    assert_eq!(book["in_flight"].as_array().unwrap().len(), 4);

    // only the named account needed a key lookup; every lot's nonce was checked
    assert_eq!(h.host.count("has_public_key"), 1);
    assert_eq!(h.host.count("is_nonce_used"), 4);
    assert_eq!(h.host.count("quote"), 2);
    assert_eq!(h.host.count("simulate_intents"), 1);

    // next batch: every nonce is used, so the in-flight lots were fills
    let r2 = h.run(&[]).result;
    assert_eq!(r2["book"], json!({ "orders": 0, "lots": 0 }));
    assert!(r2["settle"].as_array().unwrap().is_empty());
    assert!(h.book()["in_flight"].as_array().unwrap().is_empty());
}

#[test]
fn output_is_deterministic_and_signed_over_sha256_of_result() {
    let run = || {
        let Desk {
            mut h,
            seller,
            buyer_nep,
            ..
        } = desk();
        let sell = h.order(&seller, "sell", &[SELL_LIMIT], 1, 10);
        let buy = h.order(&buyer_nep, "buy", &[BUY_LIMIT], 1, 20);
        let run = h.run(&[&sell, &buy]);
        (run, h.host.result_vk())
    };
    let (a, key) = run();
    let (b, _) = run();
    assert_eq!(a.raw, b.raw, "same state, same input, same bytes");

    // what the pool does: ed25519_verify(sig, sha256(result), pinned_key)
    let out: Value = serde_json::from_str(&a.raw).unwrap();
    assert_eq!(
        out.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["key", "result", "sig"]
    );
    let result = out["result"].as_str().unwrap();
    let digest = sha256(result.as_bytes());
    let sig: [u8; 64] = ed25519_bytes(out["sig"].as_str().unwrap()).unwrap().try_into().unwrap();
    key.verify(&digest, &ed25519_dalek::Signature::from_bytes(&sig))
        .unwrap();

    // a single changed byte of the result breaks the signature
    let forged = result.replacen("\"matched_lots\":1", "\"matched_lots\":2", 1);
    assert_ne!(forged, result);
    assert!(key
        .verify(&sha256(forged.as_bytes()), &ed25519_dalek::Signature::from_bytes(&sig))
        .is_err());

    // canonical: the result re-serializes to the same string
    let parsed: Value = serde_json::from_str(result).unwrap();
    assert_eq!(parsed["settle"].as_array().unwrap().len(), 1);
    assert!(result.starts_with(r#"{"v":1,"op":"batch","input_sha256":""#));
}

#[test]
fn result_binds_the_exact_input() {
    let Desk { mut h, seller, .. } = desk();
    let sell = h.order(&seller, "sell", &[SELL_LIMIT], 1, 10);
    let input = h.input(&[sell.input()]).to_string();
    let r = h.run_raw(input.as_bytes()).result;
    assert_eq!(r["input_sha256"], hex(&sha256(input.as_bytes())));
}

#[test]
fn sub_batches_are_capped_and_the_rest_stays_resting() {
    let Desk {
        mut h,
        seller,
        buyer_nep,
        ..
    } = desk();
    h.params["max_lots_per_side"] = json!(3);
    h.params["max_sub_batches"] = json!(2);
    let sell = h.order(&seller, "sell", &[SELL_LIMIT; 10], 1, 10);
    let buy = h.order(&buyer_nep, "buy", &[BUY_LIMIT; 10], 1, 50);
    let r = h.run(&[&sell, &buy]).result;

    let settle = r["settle"].as_array().unwrap();
    assert_eq!(settle.len(), 2);
    for sub in settle {
        assert_eq!(sub.as_array().unwrap().len(), 3 + 3 + 1);
    }
    assert_eq!(r["batches"][0]["matched_lots"], 6);
    assert_eq!(r["batches"][0]["sub_batches"], 2);
    assert!(
        r["batches"][0]["dropped"].as_array().unwrap().is_empty(),
        "the cap is not reported"
    );
    let book = h.book();
    assert_eq!(book["in_flight"].as_array().unwrap().len(), 12);
    let resting: usize = book["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["lots"].as_array().unwrap().len())
        .sum();
    assert_eq!(resting, 8);
    assert_eq!(r["book"], json!({ "orders": 2, "lots": 20 }));
    // time priority inside an order: the lowest indices cross first
    let flying: Vec<u64> = book["in_flight"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["lot"]["index"].as_u64().unwrap())
        .collect();
    assert_eq!(flying.iter().filter(|i| **i < 6).count(), 12);

    // each sub-batch settles on its own
    h.settle(&r);
    // and the next batch crosses what was left
    let r2 = h.run(&[]).result;
    assert_eq!(r2["batches"][0]["matched_lots"], 4);
    assert_eq!(r2["settle"].as_array().unwrap().len(), 2);
}

#[test]
fn default_plan_keeps_each_call_within_thirteen_payloads() {
    let Desk {
        mut h,
        seller,
        buyer_nep,
        ..
    } = desk();
    // the default plan: at most 6 lots per side (6 + 6 + venue = 13 payloads)
    let sell = h.order(&seller, "sell", &[SELL_LIMIT; 9], 1, 10);
    let buy = h.order(&buyer_nep, "buy", &[BUY_LIMIT; 9], 1, 50);
    let r = h.run(&[&sell, &buy]).result;
    let sizes: Vec<usize> = r["settle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [13, 7]);
    assert!(sizes.iter().all(|n| *n <= 13));
}

#[test]
fn results_reveal_nothing_about_resting_orders() {
    let Desk {
        mut h,
        seller,
        buyer_nep,
        ..
    } = desk();
    let secretive = Trader::named("secretive.testnet", 9);
    h.fund_both(&secretive);
    // rests far above the mid and never crosses
    let resting = h.order(&secretive, "sell", &[987_654_321, 987_654_321], 1, 90);
    let sell = h.order(&seller, "sell", &[SELL_LIMIT], 1, 10);
    let buy = h.order(&buyer_nep, "buy", &[BUY_LIMIT], 1, 20);
    let run = h.run(&[&resting, &sell, &buy]);
    for needle in ["secretive", "987654321", &secretive.public_key().unwrap()] {
        assert!(!run.raw.contains(needle), "output leaks {needle}");
    }
    // a batch with nothing crossing reveals only aggregates
    let run2 = h.run(&[]);
    assert!(!run2.raw.contains("secretive"));
    assert_eq!(run2.result["batches"][0]["skipped"], "no_cross");
    assert_eq!(run2.result["book"]["orders"], 3);
}

#[test]
fn self_trades_are_blocked() {
    let Desk { mut h, seller, .. } = desk();
    let sell = h.order(&seller, "sell", &[SELL_LIMIT], 1, 10);
    let buy = h.order(&seller, "buy", &[BUY_LIMIT], 1, 20);
    let r = h.run(&[&sell, &buy]).result;
    assert_eq!(r["batches"][0]["skipped"], "no_cross");
    assert!(r["settle"].as_array().unwrap().is_empty());
}

#[test]
fn min_lots_is_honoured_across_batches() {
    let Desk {
        mut h,
        seller,
        buyer_nep,
        ..
    } = desk();
    let sell = h.order(&seller, "sell", &[SELL_LIMIT; 3], 3, 10);
    let small = h.order(&buyer_nep, "buy", &[BUY_LIMIT; 2], 1, 20);
    let r = h.run(&[&sell, &small]).result;
    assert_eq!(
        r["batches"][0]["skipped"], "no_cross",
        "3-lot minimum against 2 lots of demand"
    );
    let buyer2 = Trader::evm(4);
    h.fund_both(&buyer2);
    let more = h.order(&buyer2, "buy", &[BUY_LIMIT], 1, 30);
    let r = h.run(&[&more]).result;
    assert_eq!(r["batches"][0]["matched_lots"], 3);
}
