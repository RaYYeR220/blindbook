//! Ingestion: what enters the book, what is rejected (and with which generic
//! reason), what waits for the next batch.

mod common;

use base64::Engine as _;
use blindbook_matcher::util::{rfc3339, NS_PER_S};
use common::*;
use serde_json::{json, Value};

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn seller(h: &mut Harness) -> Trader {
    let t = Trader::named("seller.testnet", 1);
    h.fund_both(&t);
    t
}

/// Run one order alone and return the reason it was rejected, if it was.
fn verdict(h: &mut Harness, o: &Order) -> (Option<String>, Value) {
    let r = h.run(&[o]).result;
    (rejected_reason(&r, &o.id), r)
}

#[test]
fn a_valid_order_rests_in_the_book() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let o = h.order(&t, "sell", &[SELL_LIMIT, SELL_LIMIT + 1], 2, 1);
    let (reason, r) = verdict(&mut h, &o);
    assert_eq!(reason, None);
    assert_eq!(ids(&r["ingested"]), ["1"]);
    let book = h.book();
    let stored = &book["orders"][0];
    assert_eq!(stored["meta"]["owner"], t.account);
    assert_eq!(stored["meta"]["seq"], 1);
    assert_eq!(stored["meta"]["min_lots"], 2);
    assert_eq!(stored["lots"].as_array().unwrap().len(), 2);
    assert_eq!(stored["lots"][1]["limit_quote"], (SELL_LIMIT + 1).to_string());
    assert_eq!(r["book"], json!({ "orders": 1, "lots": 2 }));
}

#[test]
fn seq_comes_from_the_input() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let mut o = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    o.seq = 4_242;
    h.run(&[&o]);
    assert_eq!(h.book()["orders"][0]["meta"]["seq"], 4_242);
}

#[test]
fn tampered_ciphertext_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let mut o = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    let mut raw = base64::engine::general_purpose::STANDARD.decode(&o.sealed).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 1;
    o.sealed = b64(&raw);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
    assert!(h.book()["orders"].as_array().unwrap().is_empty());
}

#[test]
fn an_order_sealed_to_another_key_or_garbage_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let body = h.body(&t, "sell", &[SELL_LIMIT], 1, 1);
    let other = blindbook_core::envelope::MatcherKey::from_seed([0x99; 32]);
    let sealed =
        blindbook_core::envelope::seal(other.public_bytes(), body.to_string().as_bytes(), [5; 32], [6; 24]).unwrap();
    let mut o = h.order_from_body(body);
    o.sealed = b64(&sealed);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
    o.sealed = "%%% not base64 %%%".into();
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
    o.sealed = b64(b"short");
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

#[test]
fn a_tampered_lot_inside_the_envelope_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    // the trader signed 350000; the body claims a better limit in the message
    let mut body = h.body(&t, "sell", &[SELL_LIMIT], 1, 1);
    let msg = body["lots"][0]["intent"]["payload"]["message"]
        .as_str()
        .unwrap()
        .replace(&SELL_LIMIT.to_string(), "1");
    body["lots"][0]["intent"]["payload"]["message"] = json!(msg);
    body["lots"][0]["limit_quote"] = json!("1");
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

#[test]
fn the_declared_limit_must_be_the_signed_one() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let mut body = h.body(&t, "sell", &[SELL_LIMIT], 1, 1);
    body["lots"][0]["limit_quote"] = json!((SELL_LIMIT + 5).to_string());
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

#[test]
fn the_owner_must_be_the_signer_and_the_side_must_match() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let mut body = h.body(&t, "sell", &[SELL_LIMIT], 1, 1);
    body["owner"] = json!("mallory.testnet");
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));

    let mut body = h.body(&t, "sell", &[SELL_LIMIT], 1, 2);
    body["side"] = json!("buy");
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

#[test]
fn malformed_bodies_are_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    type Mutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<Mutation> = vec![
        Box::new(|b| b["v"] = json!(2)),
        Box::new(|b| b["min_lots"] = json!(0)),
        Box::new(|b| b["min_lots"] = json!(2)),
        Box::new(|b| b["lots"] = json!([])),
        Box::new(|b| b["extra"] = json!(true)),
        Box::new(|b| b["market"] = json!("other-market")),
        Box::new(|b| b["owner"] = json!("NOT AN ACCOUNT")),
        Box::new(|b| b["lots"][0]["limit_quote"] = json!(350000)),
        Box::new(|b| {
            let dup = b["lots"][0].clone();
            b["lots"].as_array_mut().unwrap().push(dup);
        }),
    ];
    for (i, mutate) in cases.iter().enumerate() {
        let mut body = h.body(&t, "sell", &[SELL_LIMIT], 1, 10 + i as u8);
        mutate(&mut body);
        let o = h.order_from_body(body);
        assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"), "case {i}");
    }
    assert!(h.book()["orders"].as_array().unwrap().is_empty());
}

#[test]
fn an_unknown_market_is_unsupported() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let o = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    let mut input = o.input();
    input["market"] = json!("doge-usdc");
    let r = h.run_raw(h.input(&[input]).to_string().as_bytes()).result;
    assert_eq!(rejected_reason(&r, &o.id).as_deref(), Some("unsupported_market"));
}

#[test]
fn an_order_expiring_within_the_margin_is_expired() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let lot = t.lot("sell", SELL_LIMIT, NOW_S + 60, 1);
    let body = json!({ "v": 1, "market": MARKET, "owner": t.account, "side": "sell", "min_lots": 1,
        "lots": [{ "index": 0, "limit_quote": SELL_LIMIT.to_string(), "intent": lot }] });
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("expired"));

    let past = t.lot("sell", SELL_LIMIT, NOW_S - 1, 2);
    let body = json!({ "v": 1, "market": MARKET, "owner": t.account, "side": "sell", "min_lots": 1,
        "lots": [{ "index": 0, "limit_quote": SELL_LIMIT.to_string(), "intent": past }] });
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("expired"));
}

#[test]
fn the_earliest_lot_sets_the_order_deadline() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let late = t.lot("sell", SELL_LIMIT, DEADLINE_S, 1);
    let soon = t.lot("sell", SELL_LIMIT, NOW_S + 30, 2);
    let body = json!({ "v": 1, "market": MARKET, "owner": t.account, "side": "sell", "min_lots": 1, "lots": [
        { "index": 0, "limit_quote": SELL_LIMIT.to_string(), "intent": late },
        { "index": 1, "limit_quote": SELL_LIMIT.to_string(), "intent": soon },
    ] });
    let o = h.order_from_body(body);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("expired"));
}

#[test]
fn a_named_account_must_have_registered_the_key() {
    let mut h = Harness::default();
    let t = Trader::named("unregistered.testnet", 7);
    // funded, but its key was never added to the verifier
    h.host
        .verifier
        .balances
        .insert((t.account.clone(), BASE.into()), 10 * NEAR);
    let o = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
    assert_eq!(h.host.count("has_public_key"), 1);
}

#[test]
fn implicit_and_evm_accounts_skip_the_key_lookup() {
    let mut h = Harness::default();
    let implicit = Trader::implicit(4);
    let evm = Trader::evm(5);
    h.fund_both(&implicit);
    h.fund_both(&evm);
    let a = h.order(&implicit, "sell", &[SELL_LIMIT], 1, 1);
    let b = h.order(&evm, "sell", &[SELL_LIMIT], 1, 2);
    let r = h.run(&[&a, &b]).result;
    assert_eq!(ids(&r["ingested"]), ["1", "2"]);
    assert_eq!(h.host.count("has_public_key"), 0);
}

#[test]
fn an_implicit_account_signing_with_a_foreign_key_needs_it_registered() {
    let mut h = Harness::default();
    let owner = Trader::implicit(4);
    // a lot for the implicit account, signed by another ed25519 key
    let other = Trader::named(&owner.account, 8);
    let o = h.order(&other, "sell", &[SELL_LIMIT], 1, 1);
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
    assert_eq!(h.host.count("has_public_key"), 1);
}

#[test]
fn a_legacy_nonce_or_one_expiring_before_the_intent_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    // nonce deadline earlier than the intent deadline
    let salt: [u8; 4] = blindbook_matcher::util::unhex(SALT).unwrap().try_into().unwrap();
    let nonce = blindbook_matcher::nonce::encode(salt, ((DEADLINE_S - 1) * NS_PER_S) as i64, [1; 15]);
    let lot = sign_with_nonce(&t, &nonce);
    let o = h.order_from_body(one_lot_body(&t, lot));
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
    // a legacy random nonce
    let lot = sign_with_nonce(&t, &[7u8; 32]);
    let o = h.order_from_body(one_lot_body(&t, lot));
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

fn one_lot_body(t: &Trader, lot: Value) -> Value {
    json!({ "v": 1, "market": MARKET, "owner": t.account, "side": "sell", "min_lots": 1,
        "lots": [{ "index": 0, "limit_quote": SELL_LIMIT.to_string(), "intent": lot }] })
}

fn sign_with_nonce(t: &Trader, nonce: &[u8; 32]) -> Value {
    use blindbook_core::intent::nep413_hash;
    use ed25519_dalek::Signer as _;
    let Signer::Nep413(key) = &t.signer else {
        panic!("nep413 trader")
    };
    let diff = json!({ BASE: format!("-{LOT}"), QUOTE: SELL_LIMIT.to_string() });
    let message = json!({ "signer_id": t.account, "deadline": rfc3339(DEADLINE_S), "intents": [{ "intent": "token_diff", "diff": diff }] }).to_string();
    let sig = key.sign(&nep413_hash(&message, nonce, VERIFIER, None));
    json!({
        "standard": "nep413",
        "payload": { "message": message, "nonce": b64(nonce), "recipient": VERIFIER },
        "public_key": t.public_key().unwrap(),
        "signature": format!("ed25519:{}", bs58::encode(sig.to_bytes()).into_string()),
    })
}

#[test]
fn an_invalid_salt_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let lot = t.sign(
        json!([{ "intent": "token_diff", "diff": { BASE: format!("-{LOT}"), QUOTE: SELL_LIMIT.to_string() } }]),
        DEADLINE_S,
        1,
        "deadbeef",
    );
    let o = h.order_from_body(one_lot_body(&t, lot));
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

#[test]
fn a_lot_whose_nonce_is_already_used_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let o = h.order(&t, "sell", &[SELL_LIMIT, SELL_LIMIT], 1, 1);
    // the trader cancelled one lot before the order was ingested
    h.host.verifier.used.insert((t.account.clone(), o.nonces()[1].clone()));
    assert_eq!(verdict(&mut h, &o).0.as_deref(), Some("invalid"));
}

#[test]
fn reusing_a_nonce_of_a_resting_order_is_invalid() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let first = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    h.run(&[&first]);
    // same nonce (same random byte), another order
    let second = h.order(&t, "sell", &[SELL_LIMIT + 7], 1, 1);
    assert_eq!(verdict(&mut h, &second).0.as_deref(), Some("invalid"));
    // and within one input
    let a = h.order(&t, "sell", &[SELL_LIMIT], 1, 50);
    let b = h.order(&t, "sell", &[SELL_LIMIT + 3], 1, 50);
    let r = h.run(&[&a, &b]).result;
    assert_eq!(ids(&r["ingested"]), std::slice::from_ref(&a.id));
    assert_eq!(rejected_reason(&r, &b.id).as_deref(), Some("invalid"));
}

#[test]
fn resending_an_ingested_order_is_idempotent() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let o = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    h.run(&[&o]);
    let r = h.run(&[&o, &o]).result;
    assert_eq!(ids(&r["ingested"]), ["1"]);
    assert_eq!(r["book"], json!({ "orders": 1, "lots": 1 }));
    assert_eq!(h.book()["orders"].as_array().unwrap().len(), 1);
}

#[test]
fn an_order_waits_when_the_verifier_cannot_be_asked() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let o = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    h.host.verifier.down.insert("has_public_key".into());
    let r = h.run(&[&o]).result;
    assert_eq!(ids(&r["deferred"]), ["1"]);
    assert!(r["ingested"].as_array().unwrap().is_empty());
    assert!(r["rejected"].as_array().unwrap().is_empty());
    // once the verifier answers, the same order is ingested
    h.host.verifier.down.clear();
    let r = h.run(&[&o]).result;
    assert_eq!(ids(&r["ingested"]), ["1"]);

    let o2 = h.order(&t, "sell", &[SELL_LIMIT], 1, 2);
    h.host.verifier.down.insert("is_nonce_used".into());
    assert_eq!(ids(&h.run(&[&o2]).result["deferred"]), ["2"]);
    h.host.verifier.down.clear();
    h.host.verifier.down.insert("is_valid_salt".into());
    assert_eq!(ids(&h.run(&[&o2]).result["deferred"]), ["2"]);
}

#[test]
fn rejection_reasons_are_generic() {
    let mut h = Harness::default();
    let t = seller(&mut h);
    let mut tampered = h.order(&t, "sell", &[SELL_LIMIT], 1, 1);
    tampered.sealed = b64(b"garbage garbage garbage garbage garbage garbage garbage garbage garbage");
    let run = h.run(&[&tampered]);
    let reasons: Vec<&str> = run.result["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["invalid"]);
    assert_eq!(
        run.result["rejected"][0].as_object().unwrap().len(),
        2,
        "id and reason only"
    );
}

#[test]
fn orders_signed_by_the_typescript_sdk_are_ingested() {
    // crates/core/tests/fixtures/sdk_orders.json: a NEP-413 implicit seller
    // and an ERC-191 buyer for wnear-usdc on intents.near.
    let fx: Value = serde_json::from_slice(
        &std::fs::read(format!(
            "{}/../crates/core/tests/fixtures/sdk_orders.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let mut h = Harness::default();
    h.host.verifier.id = "intents.near".into();
    h.host.verifier.salt = "252812b3".into();
    h.host.verifier.valid_salts.insert("252812b3".into());
    let sell = h.order_from_body(fx["sell"].clone());
    let mut buy = h.order_from_body(fx["buy"].clone());
    buy.seq = 2;
    let mut input = h.input(&[sell.input(), buy.input()]);
    input["verifier"] = json!("intents.near");
    input["markets"][0]["id"] = fx["market"]["id"].clone();
    input["markets"][0]["base"] = fx["market"]["base"].clone();
    input["markets"][0]["quote"] = fx["market"]["quote"].clone();
    input["markets"][0]["lot"] = fx["market"]["lot"].clone();
    for o in input["orders"].as_array_mut().unwrap() {
        o["market"] = fx["market"]["id"].clone();
    }
    let r = h.run_raw(input.to_string().as_bytes()).result;
    assert_eq!(ids(&r["ingested"]), ["1", "2"]);
    // the fixture's 3.54 sell limit is above the fixture quotes' midpoint
    assert_eq!(r["batches"][0]["skipped"], "no_cross");
    assert_eq!(r["book"], json!({ "orders": 2, "lots": 3 }));
}
