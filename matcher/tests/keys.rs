//! `op: "keys"`: the deployment's public keys, signed like every result.

mod common;

use blindbook_core::envelope::{seal, MatcherKey};
use blindbook_matcher::util::{ed25519_str, hex};
use common::*;
use serde_json::{json, Value};

#[test]
fn keys_op_publishes_the_order_key_venue_key_and_result_key() {
    let mut h = Harness::default();
    let run = h.run_raw(br#"{"v":1,"op":"keys"}"#);
    let r = &run.result;
    let x25519 = MatcherKey::from_seed(h.host.x25519_seed).public_bytes();
    assert_eq!(
        *r,
        json!({
            "v": 1,
            "op": "keys",
            "matcher_x25519": hex(&x25519),
            "venue": ed25519_str(&h.host.venue.verifying_key().to_bytes()),
            "result_key": ed25519_str(&h.host.result.verifying_key().to_bytes()),
        })
    );
    // exact canonical string, in the documented field order
    assert_eq!(
        run.out["result"].as_str().unwrap(),
        format!(
            r#"{{"v":1,"op":"keys","matcher_x25519":"{}","venue":"{}","result_key":"{}"}}"#,
            hex(&x25519),
            h.host.venue_pk(),
            ed25519_str(&h.host.result.verifying_key().to_bytes())
        )
    );
    // the output's `key` is the result key itself
    assert_eq!(run.out["key"], r["result_key"]);
}

#[test]
fn keys_op_ignores_orders_and_touches_nothing() {
    let mut h = Harness::default();
    let mut input = h.input(&[]);
    input["op"] = json!("keys");
    input["orders"] = json!([{ "id": "1", "seq": 1, "market": MARKET, "sealed": "AAAA" }]);
    let run = h.run_raw(input.to_string().as_bytes());
    assert_eq!(run.result["op"], "keys");
    assert!(run.result.get("ingested").is_none());
    assert!(h.host.calls.is_empty(), "no network");
    assert!(h.host.storage.is_empty(), "no storage");
}

#[test]
fn the_published_key_opens_orders_sealed_to_it() {
    let mut h = Harness::default();
    let r = h.run_raw(br#"{"v":1,"op":"keys"}"#).result;
    let pubkey: [u8; 32] = blindbook_matcher::util::unhex(r["matcher_x25519"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let sealed = seal(pubkey, b"{\"hello\":1}", [3; 32], [4; 24]).unwrap();
    assert_eq!(h.host.matcher_key().open(&sealed).unwrap(), b"{\"hello\":1}");
}

#[test]
fn keys_are_stable_across_runs_and_differ_per_deployment() {
    let mut a = Harness::default();
    let first = a.run_raw(br#"{"v":1,"op":"keys"}"#).raw;
    let second = a.run_raw(br#"{"v":1,"op":"keys"}"#).raw;
    assert_eq!(first, second);
    let mut b = Harness::default();
    b.host.x25519_seed = [0x77; 32];
    let other: Value = b.run_raw(br#"{"v":1,"op":"keys"}"#).result;
    let mine: Value = serde_json::from_str(
        serde_json::from_str::<Value>(&first).unwrap()["result"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_ne!(other["matcher_x25519"], mine["matcher_x25519"]);
}

#[test]
fn keys_op_fails_closed_without_the_keystore() {
    let mut h = Harness::default();
    h.host.keys_down = true;
    let out = blindbook_matcher::run(&mut h.host, br#"{"v":1,"op":"keys"}"#);
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap(),
        json!({ "error": "result_key_unavailable" })
    );
}
