//! Unit tests. Results are signed with a real ed25519 key (ed25519-dalek) and
//! delivered through mocked promise results, the way OutLayer resumes the yield.

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use near_sdk::mock::MockAction;
use near_sdk::serde_json::{self as sj, json, Value};
use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
use near_sdk::{bs58, test_vm_config, testing_env, PromiseResult, RuntimeFeesConfig};
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};

thread_local! {
    /// `input_sha256` of the last batch run, echoed by `batch_result`.
    static LAST_HASH: RefCell<String> = const { RefCell::new(String::new()) };
}

const NOW: u64 = 1_794_225_600_000_000_000;
const VERSION_KEY: &str = "36f855f658b9b0dbcf0781b503536443a290663df5669eaecd495954e5e30270";
const USDC: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const SMALL: usize = ENVELOPE_OVERHEAD + 2_048; // 2121
const BIG: usize = ENVELOPE_OVERHEAD + 65_536;
const YOCTO_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}
fn pool_id() -> AccountId {
    acc("pool.blindbook.near")
}
fn owner() -> AccountId {
    acc("owner.near")
}
fn alice() -> AccountId {
    acc("alice.near")
}
fn bob() -> AccountId {
    acc("bob.near")
}
fn cranker() -> AccountId {
    acc("crank.near")
}

fn result_sk() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}
fn other_sk() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}
fn key_str(sk: &SigningKey) -> String {
    format!(
        "ed25519:{}",
        bs58::encode(sk.verifying_key().as_bytes()).into_string()
    )
}

fn market() -> Market {
    Market {
        id: "wnear-usdc".into(),
        base: "nep141:wrap.near".into(),
        quote: USDC.into(),
        lot: U128(100_000_000_000_000_000_000_000),
        reference: Reference {
            base: "nep141:wrap.near".into(),
            quote: USDC.into(),
            probe_base: U128(10_000_000_000_000_000_000_000_000),
            probe_quote: U128(36_000_000),
        },
    }
}
fn params() -> Params {
    Params {
        fee_bps: 5,
        max_spread_bps: 150,
        max_lots_per_side: 6,
        max_sub_batches: 4,
        deadline_margin_s: 150,
        quote_max_age_s: 120,
    }
}

/// Test harness: one mocked chain and a block height that moves forward.
struct T {
    pool: Pool,
    height: u64,
}

fn ctx(pred: &AccountId, deposit: u128, gas: Gas, height: u64) -> VMContextBuilder {
    let mut b = VMContextBuilder::new();
    b.current_account_id(pool_id())
        .predecessor_account_id(pred.clone())
        .signer_account_id(pred.clone())
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .prepaid_gas(gas)
        .block_timestamp(NOW)
        .block_height(height)
        .account_balance(NearToken::from_near(1_000));
    b
}

fn env_as(pred: &AccountId, deposit: u128, height: u64) {
    testing_env!(ctx(pred, deposit, Gas::from_tgas(1_000), height).build());
}

fn env_callback(results: Vec<PromiseResult>, gas: Gas, height: u64) {
    testing_env!(
        ctx(&pool_id(), 0, gas, height).build(),
        test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        results
    );
}

fn new_pool(result_key: Option<String>, venue_key: Option<String>) -> Pool {
    env_as(&owner(), 0, 100);
    Pool::new(
        owner(),
        "mainnet".into(),
        acc("outlayer.near"),
        "blindbook.near/matcher".into(),
        VERSION_KEY.into(),
        result_key,
        acc("venue.blindbook.near"),
        venue_key,
        acc("intents.near"),
        vec![market()],
        params(),
    )
}

impl T {
    fn new() -> Self {
        T {
            pool: new_pool(Some(key_str(&result_sk())), None),
            height: 100,
        }
    }
    fn unpinned() -> Self {
        T {
            pool: new_pool(None, None),
            height: 100,
        }
    }

    fn submit_len(&mut self, who: &AccountId, len: usize, fill: u8) -> u64 {
        let sealed = B64.encode(vec![fill; len]);
        env_as(who, YOCTO_NEAR, self.height);
        self.pool.submit_order("wnear-usdc".into(), sealed).0
    }
    fn submit(&mut self, who: &AccountId) -> u64 {
        self.submit_len(who, SMALL, 1)
    }

    /// `run_batch` with the minimum deposit; returns (batch_id, order_ids, input).
    fn run(&mut self) -> (u64, Vec<u64>, String) {
        self.run_with(DEFAULT_RUN_DEPOSIT)
    }
    fn run_with(&mut self, deposit: u128) -> (u64, Vec<u64>, String) {
        env_as(&cranker(), deposit, self.height);
        let id = self.pool.next_batch_id;
        let _ = self.pool.run_batch();
        let (input, ids) = dispatched();
        assert_eq!(
            self.batch(id).input_hash,
            hex(&env::sha256_array(input.as_bytes()))
        );
        LAST_HASH.with(|h| *h.borrow_mut() = self.batch(id).input_hash);
        (id, ids, input)
    }
    fn run_keys(&mut self) -> u64 {
        env_as(&cranker(), DEFAULT_RUN_DEPOSIT, self.height);
        let id = self.pool.next_batch_id;
        let _ = self.pool.run_keys();
        id
    }

    fn deliver(&mut self, batch_id: u64, ids: &[u64], res: PromiseResult) -> BatchStatus {
        self.deliver_gas(
            batch_id,
            ids,
            res,
            on_result_gas(self.pool.params.max_sub_batches),
        )
    }
    fn deliver_gas(
        &mut self,
        batch_id: u64,
        ids: &[u64],
        res: PromiseResult,
        gas: Gas,
    ) -> BatchStatus {
        self.height += 10;
        env_callback(vec![res], gas, self.height);
        let _ = self.pool.on_result(batch_id, ids.to_vec());
        self.pool.batches.get(&batch_id).unwrap().status
    }
    fn settled(&mut self, batch_id: u64, results: Vec<PromiseResult>) -> BatchStatus {
        self.height += 5;
        let n = results.len() as u32;
        env_callback(results, ON_SETTLED_GAS, self.height);
        self.pool.on_settled(batch_id, n)
    }
    fn batch(&self, id: u64) -> BatchRecord {
        self.pool.batches.get(&id).cloned().unwrap()
    }
    fn pending_ids(&self) -> Vec<u64> {
        self.pool.pending.iter().map(|e| e.id).collect()
    }
}

// ---------------------------------------------------------------- receipt helpers

#[derive(Debug, Clone)]
struct Call {
    receiver: String,
    method: String,
    args: Vec<u8>,
    deposit: u128,
    gas: Gas,
}

fn calls() -> Vec<Call> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| {
            let receiver = r.receiver_id.to_string();
            r.actions.into_iter().filter_map(move |a| match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    args,
                    attached_deposit,
                    prepaid_gas,
                    ..
                } => Some(Call {
                    receiver: receiver.clone(),
                    method: String::from_utf8(method_name).unwrap(),
                    args,
                    deposit: attached_deposit.as_yoctonear(),
                    gas: prepaid_gas,
                }),
                _ => None,
            })
        })
        .collect()
}

fn transfers() -> Vec<(String, u128)> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| {
            let receiver = r.receiver_id.to_string();
            r.actions.into_iter().filter_map(move |a| match a {
                MockAction::Transfer { deposit, .. } => {
                    Some((receiver.clone(), deposit.as_yoctonear()))
                }
                _ => None,
            })
        })
        .collect()
}

/// The input and order ids of the `request_execution` just created.
fn dispatched() -> (String, Vec<u64>) {
    let cs = calls();
    let req = cs
        .iter()
        .find(|c| c.method == "request_execution")
        .expect("request_execution");
    let args: Value = sj::from_slice(&req.args).unwrap();
    let cb = cs
        .iter()
        .find(|c| c.method == "on_result")
        .expect("on_result");
    let (_, ids): (u64, Vec<u64>) = near_sdk::borsh::from_slice(&cb.args).unwrap();
    (args["input_data"].as_str().unwrap().to_string(), ids)
}

fn events(name: &str) -> Vec<Value> {
    get_logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| sj::from_str::<Value>(j).unwrap())
        .filter(|v| v["event"] == name)
        .inspect(|v| {
            assert_eq!(v["standard"], "blindbook");
            assert_eq!(v["version"], "1.0.0");
            assert!(v["data"].is_array());
        })
        .map(|v| v["data"][0].clone())
        .collect()
}

type PoolCall = Box<dyn FnOnce(&mut Pool)>;

fn panics<F: FnOnce()>(f: F) -> String {
    let err = catch_unwind(AssertUnwindSafe(f)).expect_err("expected a panic");
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

// ---------------------------------------------------------------- result builders

fn sign_with(sk: &SigningKey, claimed: &str, result: &str) -> PromiseResult {
    let digest = env::sha256_array(result.as_bytes());
    let sig = sk.sign(&digest);
    let out = json!({
        "result": result,
        "sig": format!("ed25519:{}", bs58::encode(sig.to_bytes()).into_string()),
        "key": claimed,
    });
    PromiseResult::Successful(out.to_string().into_bytes())
}
fn ok() -> PromiseResult {
    PromiseResult::Successful(vec![])
}
fn signed(result: &str) -> PromiseResult {
    sign_with(&result_sk(), &key_str(&result_sk()), result)
}

fn payload(tag: &str) -> Value {
    json!({
        "standard": "nep413",
        "payload": { "message": format!("{{\"signer_id\":\"{tag}\",\"deadline\":\"2026-11-09T14:00:00.000Z\",\"intents\":[]}}"),
                     "nonce": "VijxxgANkye...", "recipient": "intents.near" },
        "public_key": "ed25519:2Ntcp6GVLcDa64BgCwVrzHT1LYB3vLGzp9G25DcuTTdD",
        "signature": format!("ed25519:{tag}"),
    })
}

fn summary() -> Value {
    json!({ "market": "wnear-usdc", "mid": { "quote": "354000", "base": "100000000000000000000000" },
            "lot_mid_quote": "35400", "matched_lots": 2, "base_volume": "200000000000000000000000",
            "quote_volume": "70800", "rebated": "40", "venue_fee": "7", "quotes": ["q1", "q2"],
            "sub_batches": 1, "dropped": [], "skipped": null })
}

fn batch_result(ingested: &[&str], rejected: &[(&str, &str)], settle: Value) -> String {
    batch_result_h(
        &LAST_HASH.with(|h| h.borrow().clone()),
        ingested,
        rejected,
        settle,
    )
}

fn batch_result_h(
    hash: &str,
    ingested: &[&str],
    rejected: &[(&str, &str)],
    settle: Value,
) -> String {
    json!({
        "v": 1, "op": "batch", "input_sha256": hash, "now_ns": NOW.to_string(),
        "ingested": ingested, "deferred": [],
        "rejected": rejected.iter().map(|(id, r)| json!({"id": id, "reason": r})).collect::<Vec<_>>(),
        "batches": [summary()],
        "settle": settle,
        "book": { "orders": 3, "lots": 9 },
    })
    .to_string()
}

fn keys_result(matcher: &str, venue: &str, result_key: &str) -> String {
    json!({ "v": 1, "op": "keys", "matcher_x25519": matcher, "venue": venue, "result_key": result_key }).to_string()
}
const X25519: &str = "802aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7aa0e7a";
const VENUE: &str = "ed25519:2Ntcp6GVLcDa64BgCwVrzHT1LYB3vLGzp9G25DcuTTdD";

// ================================================================ init

#[test]
fn init_validates_and_normalizes() {
    let t = T::new();
    let c = t.pool.get_config();
    assert_eq!(c.result_key, Some(key_str(&result_sk())));
    assert_eq!(c.venue_key, None);
    assert_eq!(c.run_deposit, U128(DEFAULT_RUN_DEPOSIT));
    assert_eq!(c.run_gas.0, Gas::from_tgas(435).as_gas());
    assert_eq!(c.max_pending, 64);
    assert_eq!(t.pool.get_markets(), vec![market()]);

    let init = |network: &str, version: &str, key: Option<&str>, markets: Vec<Market>| {
        let (network, version, key) = (
            network.to_string(),
            version.to_string(),
            key.map(str::to_string),
        );
        panics(move || {
            env_as(&owner(), 0, 1);
            Pool::new(
                owner(),
                network,
                acc("o.near"),
                "p".into(),
                version,
                key,
                acc("v.near"),
                None,
                acc("i.near"),
                markets,
                params(),
            );
        })
    };
    assert!(init("devnet", VERSION_KEY, None, vec![]).contains("network"));
    assert!(init("mainnet", "abc", None, vec![]).contains("version_key"));
    assert!(init("mainnet", VERSION_KEY, Some("ed25519:xyz"), vec![]).contains("ed25519"));
    assert!(init("mainnet", VERSION_KEY, None, vec![market(), market()]).contains("duplicate"));
}

// ================================================================ submit_order

#[test]
fn envelope_lengths_accept_every_bucket() {
    let mut t = T::new();
    for (i, b) in BUCKETS.iter().enumerate() {
        let id = t.submit_len(&alice(), ENVELOPE_OVERHEAD + b, i as u8);
        assert_eq!(id, i as u64);
        assert_eq!(
            t.pool.get_order(U64(id)).unwrap().bytes as usize,
            ENVELOPE_OVERHEAD + b
        );
    }
    assert_eq!(t.pending_ids(), vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn envelope_lengths_reject_everything_else() {
    let mut t = T::new();
    for len in [
        0, 1, 72, 73, 2_048, 2_120, 2_122, 4_168, 4_170, 65_608, 65_610, 131_145,
    ] {
        let msg = panics(|| {
            t.submit_len(&alice(), len, 1);
        });
        assert!(msg.contains("sealed order must be"), "len {len}: {msg}");
    }
    assert!(t.pool.pending.is_empty());
}

#[test]
fn submit_rejects_bad_base64_and_unknown_market() {
    let mut t = T::new();
    env_as(&alice(), YOCTO_NEAR, 100);
    assert!(panics(|| {
        t.pool
            .submit_order("wnear-usdc".into(), "not base64!!".into());
    })
    .contains("base64"));
    env_as(&alice(), YOCTO_NEAR, 100);
    assert!(panics(|| {
        t.pool
            .submit_order("nope".into(), B64.encode(vec![0u8; SMALL]));
    })
    .contains("unknown market"));
}

#[test]
fn storage_deposit_is_measured_and_excess_refunded() {
    let mut t = T::new();
    let sealed = B64.encode(vec![3u8; SMALL]);
    let attached = YOCTO_NEAR;
    env_as(&alice(), attached, 100);
    let id = t.pool.submit_order("wnear-usdc".into(), sealed.clone()).0;

    // Storage record: 40 (per-record overhead) + key (prefix 1 + u64 8) +
    // borsh Order (4+10 market, 4+10 "alice.near", 4+2121 sealed, 16 deposit, 8 ts).
    let record = 40 + 9 + (4 + 10) + (4 + 10) + (4 + SMALL as u64) + 16 + 8;
    let required = (record + ORDER_STORAGE_BUFFER) as u128 * STORAGE_PRICE_PER_BYTE;
    let order = t.pool.get_order(U64(id)).unwrap();
    assert_eq!(order.deposit, U128(required));
    assert_eq!(order.sealed_b64.as_deref(), Some(sealed.as_str()));
    assert_eq!(
        transfers(),
        vec![("alice.near".to_string(), attached - required)]
    );
    assert!(t.pool.get_order_deposit(SMALL as u32).0 >= required);

    let ev = events("order_submitted");
    assert_eq!(ev.len(), 1);
    assert_eq!(
        ev[0],
        json!({"id": "0", "seq": 0, "market": "wnear-usdc", "submitter": "alice.near", "bytes": SMALL, "deposit": required.to_string()})
    );
    let log = get_logs()
        .into_iter()
        .find(|l| l.contains("order_submitted"))
        .unwrap();
    assert!(log.starts_with(
        r#"EVENT_JSON:{"standard":"blindbook","version":"1.0.0","event":"order_submitted","data":[{"id":"0","#
    ));

    // Exactly the required amount is enough; one yocto less is not.
    // "bob.near" is two bytes shorter than "alice.near".
    let required = required - 2 * STORAGE_PRICE_PER_BYTE;
    env_as(&bob(), required, 100);
    t.pool.submit_order("wnear-usdc".into(), sealed.clone());
    assert!(transfers().is_empty());
    env_as(&bob(), required - 1, 100);
    assert!(panics(|| {
        t.pool.submit_order("wnear-usdc".into(), sealed.clone());
    })
    .contains("attach at least"));
}

#[test]
fn order_deposit_view_covers_every_bucket_and_long_account_ids() {
    let mut t = T::new();
    let long = acc(&format!("{}.near", "a".repeat(59))); // 64 chars, the maximum
    for b in BUCKETS {
        let len = ENVELOPE_OVERHEAD + b;
        let quote = t.pool.get_order_deposit(len as u32).0;
        env_as(&long, quote, 100);
        let id = t
            .pool
            .submit_order("wnear-usdc".into(), B64.encode(vec![0u8; len]))
            .0;
        assert!(t.pool.get_order(U64(id)).unwrap().deposit.0 <= quote);
    }
}

#[test]
fn pending_queue_is_capped() {
    let mut t = T::new();
    for _ in 0..MAX_PENDING {
        t.submit(&alice());
    }
    assert!(panics(|| {
        t.submit(&alice());
    })
    .contains("full"));
}

// ================================================================ cancel_order

#[test]
fn cancel_refunds_submitter_only() {
    let mut t = T::new();
    let id = t.submit(&alice());
    let deposit = t.pool.get_order(U64(id)).unwrap().deposit.0;

    env_as(&bob(), 0, 100);
    assert!(panics(|| {
        t.pool.cancel_order(U64(id));
    })
    .contains("only the submitter"));

    env_as(&alice(), 0, 100);
    assert_eq!(t.pool.cancel_order(U64(id)), U128(deposit));
    assert_eq!(transfers(), vec![("alice.near".to_string(), deposit)]);
    assert_eq!(
        events("order_cancelled")[0],
        json!({"id": "0", "submitter": "alice.near", "refund": deposit.to_string()})
    );
    assert!(t.pool.get_order(U64(id)).is_none());
    assert!(t.pool.pending.is_empty());

    env_as(&alice(), 0, 100);
    assert!(panics(|| {
        t.pool.cancel_order(U64(id));
    })
    .contains("not pending"));
    env_as(&alice(), 0, 100);
    assert!(panics(|| {
        t.pool.cancel_order(U64(42));
    })
    .contains("not pending"));
}

#[test]
fn cancel_blocked_while_in_flight_until_guard_expires() {
    let mut t = T::new();
    let a = t.submit(&alice());
    let (_, ids, _) = t.run();
    assert_eq!(ids, vec![a]);
    let b = t.submit(&alice()); // submitted after the batch left: cancellable

    assert!(t.pool.get_pending(None, None)[0].in_flight);
    env_as(&alice(), 0, t.height + 1);
    assert!(panics(|| {
        t.pool.cancel_order(U64(a));
    })
    .contains("running batch"));
    env_as(&alice(), 0, t.height + 1);
    t.pool.cancel_order(U64(b));

    env_as(&alice(), 0, t.height + GUARD_TIMEOUT_BLOCKS + 1);
    assert!(!t.pool.get_pending(None, None)[0].in_flight);
    t.pool.cancel_order(U64(a));
    assert!(t.pool.pending.is_empty());
}

// ================================================================ run_batch: input

#[test]
fn run_batch_builds_the_protocol_input_exactly() {
    let mut t = T::new();
    let sealed = vec![0xabu8; SMALL];
    env_as(&alice(), YOCTO_NEAR, 100);
    t.pool
        .submit_order("wnear-usdc".into(), B64.encode(&sealed));

    env_as(&cranker(), DEFAULT_RUN_DEPOSIT, 100);
    let _ = t.pool.run_batch();
    let expected = format!(
        concat!(
            r#"{{"v":1,"op":"batch","network":"mainnet","verifier":"intents.near","venue_account":"venue.blindbook.near","#,
            r#""now_ns":"1794225600000000000","markets":[{{"id":"wnear-usdc","base":"nep141:wrap.near","quote":"{usdc}","#,
            r#""lot":"100000000000000000000000","reference":{{"base":"nep141:wrap.near","quote":"{usdc}","#,
            r#""probe_base":"10000000000000000000000000","probe_quote":"36000000"}}}}],"#,
            r#""params":{{"fee_bps":5,"max_spread_bps":150,"max_lots_per_side":6,"max_sub_batches":4,"deadline_margin_s":150,"quote_max_age_s":120}},"#,
            r#""orders":[{{"id":"0","seq":0,"market":"wnear-usdc","sealed":"{b64}"}}]}}"#
        ),
        usdc = USDC,
        b64 = B64.encode(&sealed)
    );

    let cs = calls();
    assert_eq!(cs.len(), 2);
    let req = &cs[0];
    assert_eq!(req.receiver, "outlayer.near");
    assert_eq!(req.method, "request_execution");
    assert_eq!(req.deposit, DEFAULT_RUN_DEPOSIT);
    assert_eq!(req.gas, REQUEST_GAS);
    let args: Value = sj::from_slice(&req.args).unwrap();
    assert_eq!(args["input_data"].as_str().unwrap(), expected);
    assert_eq!(
        args,
        json!({
            "source": { "Project": { "project_id": "blindbook.near/matcher", "version_key": VERSION_KEY } },
            "resource_limits": { "max_instructions": 20_000_000_000u64, "max_memory_mb": 256, "max_execution_seconds": 30 },
            "input_data": expected,
            "response_format": "Json",
            "payer_account_id": "crank.near",
        })
    );
    let cb = &cs[1];
    assert_eq!(cb.receiver, "pool.blindbook.near");
    assert_eq!(cb.method, "on_result");
    assert_eq!(cb.gas, on_result_gas(4));
    assert_eq!(
        near_sdk::borsh::from_slice::<(u64, Vec<u64>)>(&cb.args).unwrap(),
        (0, vec![0])
    );

    let rec = t.batch(0);
    assert_eq!(rec.status, BatchStatus::Requested);
    assert_eq!(rec.orders, vec![U64(0)]);
    assert_eq!(rec.input_bytes as usize, expected.len());
    assert_eq!(rec.cranker, cranker());
    assert_eq!(
        events("batch_requested")[0],
        json!({"batch_id": "0", "op": "batch", "cranker": "crank.near", "orders": ["0"], "input_bytes": expected.len(), "deposit": DEFAULT_RUN_DEPOSIT.to_string()})
    );
    assert_eq!(t.pool.get_config().in_flight.unwrap().orders, vec![0]);
}

#[test]
fn run_keys_sends_no_orders() {
    let mut t = T::new();
    t.submit(&alice());
    t.run_keys();
    let (input, ids) = dispatched();
    assert!(ids.is_empty());
    let v: Value = sj::from_str(&input).unwrap();
    assert_eq!(v["op"], "keys");
    assert_eq!(v["orders"], json!([]));
    assert_eq!(v["markets"][0]["id"], "wnear-usdc");
    assert_eq!(t.pending_ids(), vec![0]);
}

#[test]
fn input_order_length_formula_is_exact() {
    for (id, market, len) in [
        (0u64, "m", 1usize),
        (9, "wnear-usdc", SMALL),
        (10, "x", 2),
        (123_456, "a-b.c_d", 4_169),
    ] {
        let item = sj::to_string(&InputOrder {
            id: id.to_string(),
            seq: id,
            market: market.into(),
            sealed: B64.encode(vec![0u8; len]),
        })
        .unwrap();
        assert_eq!(item.len(), input_order_len(id, market.len(), len), "{item}");
    }
}

#[test]
fn input_fits_the_free_budget() {
    let mut t = T::new();
    for _ in 0..12 {
        t.submit(&alice());
    }
    let (_, ids, input) = t.run();
    assert_eq!(ids, (0..10).collect::<Vec<_>>());
    assert!(input.len() <= SOFT_INPUT_BYTES, "{}", input.len());
    // Tight: one more small order would not have fitted.
    assert!(input.len() + 1 + input_order_len(10, 10, SMALL) > SOFT_INPUT_BYTES);
    assert_eq!(
        sj::from_str::<Value>(&input).unwrap()["orders"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
}

#[test]
fn larger_deposit_buys_a_larger_input_and_big_orders_wait() {
    let mut t = T::new();
    let big = t.submit_len(&alice(), BIG, 5);
    let small = t.submit(&bob());

    // Minimum deposit: the 64 KiB order does not fit and is skipped, not blocking.
    let (b0, ids, _) = t.run();
    assert_eq!(ids, vec![small]);
    t.deliver(b0, &ids, signed(&batch_result(&["1"], &[], json!([]))));
    assert_eq!(t.pending_ids(), vec![big]);

    // Pay for the bytes above OutLayer's free 32 KiB and it goes.
    let (_, ids, input) = t.run_with(DEFAULT_RUN_DEPOSIT + YOCTO_NEAR);
    assert_eq!(ids, vec![big]);
    assert!(input.len() > OUTLAYER_FREE_INPUT);
    assert!(input_storage_deposit(input.len()) <= YOCTO_NEAR);
}

#[test]
fn input_holds_at_most_max_orders() {
    let mut t = T::new();
    for _ in 0..40 {
        t.submit(&alice());
    }
    let (_, ids, input) = t.run_with(DEFAULT_RUN_DEPOSIT + YOCTO_NEAR);
    assert_eq!(ids.len(), MAX_ORDERS_PER_BATCH);
    assert!(input.len() <= MAX_INPUT_BYTES);
}

#[test]
fn run_batch_requires_deposit_and_gas() {
    let mut t = T::new();
    env_as(&cranker(), DEFAULT_RUN_DEPOSIT - 1, 100);
    assert!(panics(|| {
        let _ = t.pool.run_batch();
    })
    .contains("attach at least"));

    testing_env!(ctx(
        &cranker(),
        DEFAULT_RUN_DEPOSIT,
        Gas::from_gas(run_gas(4).as_gas() - 1),
        100
    )
    .build());
    assert!(panics(|| {
        let _ = t.pool.run_batch();
    })
    .contains("attach at least 435 TGas"));

    testing_env!(ctx(&cranker(), DEFAULT_RUN_DEPOSIT, run_gas(4), 100).build());
    let _ = t.pool.run_batch();
}

#[test]
fn run_gas_tracks_max_sub_batches() {
    assert_eq!(run_gas(1).as_tgas(), 270);
    assert_eq!(run_gas(4).as_tgas(), 435);
    assert_eq!(run_gas(MAX_SUB_BATCHES_CAP).as_tgas(), 875);
    assert!(run_gas(MAX_SUB_BATCHES_CAP) <= Gas::from_tgas(1_000));
}

// ================================================================ guard

#[test]
fn guard_blocks_overlapping_runs_and_times_out() {
    let mut t = T::new();
    t.submit(&alice());
    let (b0, ids0, _) = t.run();

    env_as(
        &cranker(),
        DEFAULT_RUN_DEPOSIT,
        t.height + GUARD_TIMEOUT_BLOCKS,
    );
    assert!(panics(|| {
        let _ = t.pool.run_batch();
    })
    .contains("already running"));
    env_as(&cranker(), DEFAULT_RUN_DEPOSIT, t.height);
    assert!(panics(|| {
        let _ = t.pool.run_keys();
    })
    .contains("already running"));

    // Past the timeout another crank may take over; the stuck run is marked expired.
    t.height += GUARD_TIMEOUT_BLOCKS + 1;
    let (b1, ids1, _) = t.run();
    assert_eq!(ids1, ids0);
    assert_eq!(t.batch(b0).status, BatchStatus::Expired);
    assert_eq!(t.pool.get_config().in_flight.unwrap().batch_id, b1);

    // The late result still lands but leaves the new run's guard alone.
    let h0 = t.batch(b0).input_hash;
    t.deliver(
        b0,
        &ids0,
        signed(&batch_result_h(&h0, &["0"], &[], json!([]))),
    );
    assert_eq!(t.batch(b0).status, BatchStatus::Completed);
    assert_eq!(t.pool.get_config().in_flight.unwrap().batch_id, b1);
    assert!(t.pending_ids().is_empty());

    // ...and the new run's result does not double-refund.
    t.deliver(b1, &ids1, signed(&batch_result(&["0"], &[], json!([]))));
    assert!(transfers().is_empty());
    assert_eq!(t.batch(b1).ingested, vec![]);
    assert!(t.pool.get_config().in_flight.is_none());
}

#[test]
fn guard_clears_on_every_outcome() {
    let mut t = T::new();
    let cases: Vec<fn() -> PromiseResult> = vec![
        || PromiseResult::Successful(b"null".to_vec()),
        || PromiseResult::Failed,
        || PromiseResult::Successful(b"garbage".to_vec()),
        || {
            sign_with(
                &other_sk(),
                &key_str(&result_sk()),
                &batch_result(&[], &[], json!([])),
            )
        },
        || signed(&batch_result(&[], &[], json!([]))),
    ];
    for res in cases {
        let (b, ids, _) = t.run();
        t.deliver(b, &ids, res());
        assert!(t.pool.get_config().in_flight.is_none());
    }
    assert_eq!(t.pool.get_stats().totals.batches, 5);
    assert_eq!(t.pool.get_stats().totals.failed, 4);
}

// ================================================================ on_result: batch

#[test]
fn valid_result_refunds_records_and_settles() {
    let mut t = T::new();
    let o0 = t.submit(&alice());
    let o1 = t.submit(&bob());
    let o2 = t.submit(&alice());
    let deposit = |id: u64, t: &T| t.pool.get_order(U64(id)).unwrap().deposit.0;
    let (d0, d1, d2) = (deposit(o0, &t), deposit(o1, &t), deposit(o2, &t));
    let (b, ids, _) = t.run();
    assert_eq!(ids, vec![o0, o1, o2]);
    let late = t.submit(&bob()); // not in the batch

    let settle = json!([
        [payload("s1"), payload("b1"), payload("v1")],
        [payload("s2"), payload("v2")]
    ]);
    let result = batch_result(&["0", "1"], &[("2", "invalid")], settle.clone());
    let status = t.deliver(b, &ids, signed(&result));
    assert_eq!(status, BatchStatus::Settling);

    // Orders out of the queue, deposits back, one transfer per submitter.
    assert_eq!(t.pending_ids(), vec![late]);
    let mut tr = transfers();
    tr.sort();
    assert_eq!(
        tr,
        vec![
            ("alice.near".to_string(), d0 + d2),
            ("bob.near".to_string(), d1)
        ]
    );

    // One execute_intents per sub-batch, payloads passed through byte for byte.
    let cs = calls();
    let settles: Vec<_> = cs
        .iter()
        .filter(|c| c.method == "execute_intents")
        .collect();
    assert_eq!(settles.len(), 2);
    for (i, c) in settles.iter().enumerate() {
        assert_eq!(c.receiver, "intents.near");
        assert_eq!(c.deposit, 0);
        assert_eq!(c.gas, SETTLE_GAS);
        assert_eq!(
            String::from_utf8(c.args.clone()).unwrap(),
            json!({"signed": settle[i]}).to_string()
        );
    }
    let cb = cs.iter().find(|c| c.method == "on_settled").unwrap();
    assert_eq!(
        near_sdk::borsh::from_slice::<(u64, u32)>(&cb.args).unwrap(),
        (b, 2)
    );
    assert_eq!(cb.gas, ON_SETTLED_GAS);

    let rec = t.batch(b);
    assert_eq!(rec.ingested, vec![U64(0), U64(1)]);
    assert_eq!(
        rec.rejected,
        vec![Rejection {
            id: U64(2),
            reason: "invalid".into()
        }]
    );
    assert_eq!(rec.refunded, U128(d0 + d1 + d2));
    assert_eq!(rec.sub_batches, 2);
    assert_eq!(rec.dispatched, 2);
    assert_eq!(rec.book, Some(BookCounts { orders: 3, lots: 9 }));
    assert_eq!(
        rec.result_hash,
        Some(hex(&env::sha256_array(result.as_bytes())))
    );
    assert_eq!(rec.error, None);
    let m = &rec.markets[0];
    assert_eq!(m.market, "wnear-usdc");
    assert_eq!(m.matched_lots, 2);
    assert_eq!(m.quote_volume, U128(70_800));
    assert_eq!(m.venue_fee, U128(7));
    assert_eq!(m.quotes, vec!["q1".to_string(), "q2".to_string()]);
    assert_eq!(
        m.mid,
        Some(Mid {
            quote: "354000".into(),
            base: "100000000000000000000000".into()
        })
    );
    assert_eq!(m.skipped, None);

    let ev = &events("batch_result")[0];
    assert_eq!(ev["status"], "settling");
    assert_eq!(ev["ingested"], json!(["0", "1"]));
    assert_eq!(ev["rejected"], json!([{"id": "2", "reason": "invalid"}]));
    assert_eq!(ev["markets"][0]["matched_lots"], 2);
    assert!(t.pool.get_config().in_flight.is_none());
    let totals = t.pool.get_stats().totals;
    assert_eq!((totals.orders_ingested, totals.orders_rejected), (2, 1));
}

#[test]
fn settlement_outcomes_and_stats() {
    let mut t = T::new();
    let settle2 = json!([[payload("a")], [payload("b")]]);

    // All sub-batches settle: stats count the batch.
    t.submit(&alice());
    let (b, ids, _) = t.run();
    t.deliver(b, &ids, signed(&batch_result(&["0"], &[], settle2.clone())));
    assert_eq!(t.settled(b, vec![ok(), ok()]), BatchStatus::Settled);
    assert_eq!(t.batch(b).settled, vec![true, true]);
    assert_eq!(
        events("batch_settled")[0],
        json!({"batch_id": b.to_string(), "status": "settled", "sub_batches": 2, "settled": 2, "failed": 0, "results": [true, true]})
    );
    let s = t.pool.get_stats();
    assert_eq!(
        s.markets,
        vec![MarketStats {
            market: "wnear-usdc".into(),
            batches: 1,
            crossed_lots: 2,
            base_volume: U128(200_000_000_000_000_000_000_000),
            quote_volume: U128(70_800),
            venue_fees: U128(7),
        }]
    );

    // One fails: partially settled, volumes not counted (no market/sub-batch map).
    let (b, ids, _) = t.run();
    t.deliver(b, &ids, signed(&batch_result(&[], &[], settle2.clone())));
    assert_eq!(
        t.settled(b, vec![ok(), PromiseResult::Failed]),
        BatchStatus::PartiallySettled
    );
    assert_eq!(t.batch(b).settled, vec![true, false]);

    // All fail.
    let (b, ids, _) = t.run();
    t.deliver(b, &ids, signed(&batch_result(&[], &[], settle2)));
    assert_eq!(
        t.settled(b, vec![PromiseResult::Failed, PromiseResult::Failed]),
        BatchStatus::SettlementFailed
    );

    let s = t.pool.get_stats();
    assert_eq!(s.markets[0].batches, 1);
    assert_eq!(s.markets[0].crossed_lots, 2);
    assert_eq!(
        (s.totals.sub_batches_settled, s.totals.sub_batches_failed),
        (3, 3)
    );
    assert_eq!(s.totals.batches, 3);
}

#[test]
fn settle_is_capped_by_max_sub_batches_and_gas() {
    let mut t = T::new();
    env_as(&owner(), 0, 100);
    t.pool.set_params(Params {
        max_sub_batches: 2,
        ..params()
    });
    let three = json!([[payload("a")], [payload("b")], [payload("c")], []]);

    let (b, ids, _) = t.run();
    t.deliver(b, &ids, signed(&batch_result(&[], &[], three.clone())));
    let rec = t.batch(b);
    assert_eq!((rec.sub_batches, rec.dispatched), (3, 2)); // the empty one is dropped
    assert_eq!(rec.error.as_deref(), Some("1 of 3 sub-batches not sent"));
    assert_eq!(
        calls()
            .iter()
            .filter(|c| c.method == "execute_intents")
            .count(),
        2
    );
    assert_eq!(
        t.settled(b, vec![ok(), ok()]),
        BatchStatus::PartiallySettled
    );

    // A callback with less gas than promised sends only what it can afford.
    let (b, ids, _) = t.run();
    t.deliver_gas(
        b,
        &ids,
        signed(&batch_result(&[], &[], three)),
        on_result_gas(1),
    );
    assert_eq!(t.batch(b).dispatched, 1);
    assert_eq!(
        calls()
            .iter()
            .filter(|c| c.method == "execute_intents")
            .count(),
        1
    );
}

#[test]
fn empty_settle_completes_without_calls() {
    let mut t = T::new();
    t.submit(&alice());
    let (b, ids, _) = t.run();
    let h = t.batch(b).input_hash;
    let r = json!({"v": 1, "op": "batch", "input_sha256": h, "ingested": ["0"], "rejected": [], "batches": [], "settle": [], "book": null}).to_string();
    assert_eq!(t.deliver(b, &ids, signed(&r)), BatchStatus::Completed);
    assert!(calls().is_empty());
    assert_eq!(transfers().len(), 1);
}

#[test]
fn ids_outside_the_batch_are_ignored() {
    let mut t = T::new();
    let o0 = t.submit(&alice());
    let (b, ids, _) = t.run();
    let o1 = t.submit(&alice());
    t.deliver(
        b,
        &ids,
        signed(&batch_result(
            &["1", "99", "x"],
            &[("7", "invalid")],
            json!([]),
        )),
    );
    assert_eq!(t.pending_ids(), vec![o0, o1]);
    assert!(transfers().is_empty());
    let rec = t.batch(b);
    assert!(rec.ingested.is_empty() && rec.rejected.is_empty());
}

#[test]
fn summaries_and_reasons_are_bounded() {
    let mut t = T::new();
    t.submit(&alice());
    let (b, ids, _) = t.run();
    let long = "r".repeat(500);
    let mut s = summary();
    s["quotes"] = json!(["1", "2", "3", "4", "5", "6"]);
    s["skipped"] = json!({"why": "spread"});
    let r = json!({"v": 1, "op": "batch", "input_sha256": t.batch(b).input_hash, "ingested": [],
                   "rejected": [{"id": "0", "reason": long}], "batches": vec![s; 20], "settle": []})
    .to_string();
    t.deliver(b, &ids, signed(&r));
    let rec = t.batch(b);
    assert_eq!(rec.rejected[0].reason.len(), MAX_REASON_LEN);
    assert_eq!(rec.markets.len(), MAX_MARKETS);
    assert_eq!(rec.markets[0].sub_batches, 1);
    assert_eq!(rec.markets[0].quotes.len(), 4);
    assert_eq!(
        rec.markets[0].skipped.as_deref(),
        Some(r#"{"why":"spread"}"#)
    );
}

// ================================================================ on_result: failures

#[test]
fn bad_signature_is_rejected_and_orders_kept() {
    let mut t = T::new();
    t.submit(&alice());
    let good = batch_result(&["0"], &[], json!([[payload("a")]]));
    let tampered = {
        let PromiseResult::Successful(b) = signed(&good) else {
            unreachable!()
        };
        let mut v: Value = sj::from_slice(&b).unwrap();
        v["result"] = json!(good.replace("\"0\"", "\"1\""));
        PromiseResult::Successful(v.to_string().into_bytes())
    };
    let cases: Vec<(PromiseResult, &str)> = vec![
        // Signed by another key while claiming the pinned one.
        (
            sign_with(&other_sk(), &key_str(&result_sk()), &good),
            "signature does not verify",
        ),
        // Honest about the other key.
        (
            sign_with(&other_sk(), &key_str(&other_sk()), &good),
            "result signed for another key",
        ),
        // Tampered after signing.
        (tampered, "signature does not verify"),
        (
            PromiseResult::Successful(
                json!({"result": good, "sig": "ed25519:abc", "key": key_str(&result_sk())})
                    .to_string()
                    .into_bytes(),
            ),
            "malformed signature",
        ),
    ];
    for (res, why) in cases {
        let (b, ids, _) = t.run();
        assert_eq!(
            t.deliver(b, &ids, res),
            BatchStatus::RejectedSignature,
            "{why}"
        );
        assert_eq!(t.batch(b).error.as_deref(), Some(why));
        assert_eq!(t.pending_ids(), vec![0]);
        assert!(calls().is_empty() && transfers().is_empty());
        assert_eq!(events("batch_result")[0]["status"], "rejected_signature");
    }
}

#[test]
fn signature_without_key_field_still_verifies() {
    let mut t = T::new();
    t.submit(&alice());
    let (b, ids, _) = t.run();
    let r = batch_result(&["0"], &[], json!([]));
    let digest = env::sha256_array(r.as_bytes());
    let sig = result_sk().sign(&digest);
    let out = json!({"result": r, "sig": format!("ed25519:{}", bs58::encode(sig.to_bytes()).into_string())});
    assert_eq!(
        t.deliver(
            b,
            &ids,
            PromiseResult::Successful(out.to_string().into_bytes())
        ),
        BatchStatus::Completed
    );
}

#[test]
fn null_and_failed_runs_keep_orders() {
    let mut t = T::new();
    t.submit(&alice());

    let (b, ids, _) = t.run();
    assert_eq!(
        t.deliver(b, &ids, PromiseResult::Successful(b" null\n".to_vec())),
        BatchStatus::Failed
    );
    assert_eq!(t.batch(b).error.as_deref(), Some("matcher run failed"));
    assert!(transfers().is_empty());
    assert_eq!(t.pending_ids(), vec![0]);

    // The OutLayer call itself failed: its deposit came back to the pool, pass it on.
    let (b, ids, _) = t.run();
    assert_eq!(
        t.deliver(b, &ids, PromiseResult::Failed),
        BatchStatus::Failed
    );
    assert_eq!(
        transfers(),
        vec![("crank.near".to_string(), DEFAULT_RUN_DEPOSIT)]
    );
    assert_eq!(t.pending_ids(), vec![0]);

    let (b, ids, _) = t.run();
    let huge = PromiseResult::Successful(vec![b' '; MAX_RESULT_BYTES + 1]);
    assert_eq!(t.deliver(b, &ids, huge), BatchStatus::Failed);
    assert_eq!(t.batch(b).error.as_deref(), Some("result too long"));
}

#[test]
fn malformed_results_are_invalid() {
    let mut t = T::new();
    t.submit(&alice());
    let cases: Vec<(PromiseResult, &str)> = vec![
        (
            PromiseResult::Successful(b"{\"rezult\":\"x\"}".to_vec()),
            "output is not {result, sig, key}",
        ),
        (
            PromiseResult::Successful(b"[1]".to_vec()),
            "output is not {result, sig, key}",
        ),
        (signed("not json"), "malformed batch result"),
        (
            signed(&json!({"v": 1, "op": "batch", "input_sha256": "00"}).to_string()),
            "result answers another input",
        ),
        (
            signed(&json!({"v": 1, "op": "batch"}).to_string()),
            "result answers another input",
        ),
        (
            signed(&json!({"v": 2, "op": "batch"}).to_string()),
            "result v/op does not match the request",
        ),
        (
            signed(&keys_result(X25519, VENUE, &key_str(&result_sk()))),
            "result v/op does not match the request",
        ),
        (
            signed(&json!({"v": 1, "op": "batch", "ingested": "0"}).to_string()),
            "malformed batch result",
        ),
        (
            signed(&json!({"v": 1, "op": "batch", "deferred": [0]}).to_string()),
            "malformed batch result",
        ),
        (
            signed(
                &json!({"v": 1, "op": "batch", "batches": [{"market": "m", "base_volume": "1.5"}]})
                    .to_string(),
            ),
            "malformed batch result",
        ),
    ];
    for (res, why) in cases {
        let (b, ids, _) = t.run();
        assert_eq!(t.deliver(b, &ids, res), BatchStatus::InvalidResult, "{why}");
        assert_eq!(t.batch(b).error.as_deref(), Some(why));
        assert_eq!(t.pending_ids(), vec![0]);
    }
}

// ================================================================ keys

#[test]
fn keys_run_publishes_matcher_key_and_learns_venue() {
    let mut t = T::new();
    assert_eq!(t.pool.get_matcher_key(), None);
    let b = t.run_keys();
    let upper = X25519.to_ascii_uppercase();
    assert_eq!(
        t.deliver(
            b,
            &[],
            signed(&keys_result(&upper, VENUE, &key_str(&result_sk())))
        ),
        BatchStatus::Completed
    );
    assert_eq!(t.pool.get_matcher_key(), Some(X25519.to_string()));
    assert_eq!(t.pool.get_config().venue_key, Some(VENUE.to_string()));
    assert_eq!(
        events("keys_published")[0],
        json!({"batch_id": b.to_string(), "matcher_x25519": X25519, "venue": VENUE, "result_key": key_str(&result_sk())})
    );
    assert_eq!(t.batch(b).keys.unwrap().venue, VENUE);
}

#[test]
fn keys_run_checks_pinned_keys() {
    let mut t = T::new();
    env_as(&owner(), 0, 100);
    t.pool
        .set_venue(acc("venue.blindbook.near"), Some(key_str(&other_sk())));
    let cases = [
        (
            keys_result(X25519, VENUE, &key_str(&result_sk())),
            "venue key differs from the pinned key",
        ),
        (
            keys_result(X25519, &key_str(&other_sk()), &key_str(&other_sk())),
            "result_key differs from the pinned key",
        ),
        (
            keys_result("zz", &key_str(&other_sk()), &key_str(&result_sk())),
            "matcher_x25519 is not 32 bytes of hex",
        ),
        (
            keys_result(X25519, "ed25519:nope", &key_str(&result_sk())),
            "venue or result_key is not an ed25519 key",
        ),
        (
            json!({"v": 1, "op": "keys"}).to_string(),
            "matcher_x25519 is not 32 bytes of hex",
        ),
        (
            json!({"v": 1, "op": "keys", "venue": 5}).to_string(),
            "malformed keys result",
        ),
    ];
    for (r, why) in cases {
        let b = t.run_keys();
        assert_eq!(
            t.deliver(b, &[], signed(&r)),
            BatchStatus::InvalidResult,
            "{why}"
        );
        assert_eq!(t.batch(b).error.as_deref(), Some(why));
        assert_eq!(t.pool.get_matcher_key(), None);
    }
    // Pinned venue matches: accepted.
    let b = t.run_keys();
    let ok = keys_result(X25519, &key_str(&other_sk()), &key_str(&result_sk()));
    assert_eq!(t.deliver(b, &[], signed(&ok)), BatchStatus::Completed);
    assert_eq!(t.pool.get_matcher_key(), Some(X25519.to_string()));
}

#[test]
fn unpinned_pool_records_keys_but_trusts_nothing() {
    let mut t = T::unpinned();
    let b = t.run_keys();
    let claimed = keys_result(X25519, VENUE, &key_str(&result_sk()));
    assert_eq!(t.deliver(b, &[], signed(&claimed)), BatchStatus::Unpinned);
    let rec = t.batch(b);
    assert_eq!(rec.keys.unwrap().result_key, key_str(&result_sk()));
    assert_eq!(t.pool.get_matcher_key(), None);
    assert_eq!(t.pool.get_config().venue_key, None);

    // Batches are refused until a key is pinned.
    t.submit(&alice());
    let (b, ids, _) = t.run();
    assert_eq!(
        t.deliver(b, &ids, signed(&batch_result(&["0"], &[], json!([])))),
        BatchStatus::RejectedSignature
    );
    assert_eq!(t.batch(b).error.as_deref(), Some("no result key pinned"));

    // The owner pins the key (after checking the attestation); the next keys run verifies.
    env_as(&owner(), 0, t.height);
    t.pool.set_result_key(key_str(&result_sk()));
    let b = t.run_keys();
    assert_eq!(t.deliver(b, &[], signed(&claimed)), BatchStatus::Completed);
    assert_eq!(t.pool.get_matcher_key(), Some(X25519.to_string()));
}

// ================================================================ owner

#[test]
fn owner_methods_are_owner_only() {
    let mut t = T::new();
    let cases: Vec<PoolCall> = vec![
        Box::new(|p| p.set_project("x/y".into(), VERSION_KEY.into())),
        Box::new(|p| p.set_result_key(key_str(&other_sk()))),
        Box::new(|p| p.set_venue(bob(), None)),
        Box::new(|p| {
            p.add_market(Market {
                id: "m2".into(),
                ..market()
            })
        }),
        Box::new(|p| p.remove_market("wnear-usdc".into())),
        Box::new(|p| p.set_params(params())),
        Box::new(|p| p.set_run_deposit(U128(1))),
    ];
    for f in cases {
        env_as(&alice(), 0, 100);
        let p = &mut t.pool;
        assert!(panics(|| f(p)).contains("owner only"));
    }
}

#[test]
fn owner_changes_emit_events_and_validate() {
    let mut t = T::new();
    env_as(&owner(), 0, 100);
    let v2 = "b".repeat(64);
    t.pool
        .set_project("blindbook.near/matcher".into(), v2.clone());
    t.pool.set_result_key(key_str(&other_sk()));
    t.pool.set_venue(bob(), Some(VENUE.into()));
    t.pool.add_market(Market {
        id: "eth-usdc".into(),
        base: "nep141:eth.omft.near".into(),
        ..market()
    });
    t.pool.set_params(Params {
        fee_bps: 3,
        ..params()
    });
    t.pool.set_run_deposit(U128(4_002_000_000_000_000_000_000));
    t.pool.remove_market("wnear-usdc".into());
    let changed = events("config_changed");
    let fields: Vec<&str> = changed
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(
        fields,
        [
            "project",
            "result_key",
            "venue",
            "market_added",
            "params",
            "run_deposit",
            "market_removed"
        ]
    );
    assert!(changed.iter().all(|e| e["by"] == "owner.near"));
    assert_eq!(changed[0]["value"], json!(["blindbook.near/matcher", v2]));

    let c = t.pool.get_config();
    assert_eq!(c.version_key, v2);
    assert_eq!(c.result_key, Some(key_str(&other_sk())));
    assert_eq!(
        (c.venue_account, c.venue_key),
        (bob(), Some(VENUE.to_string()))
    );
    assert_eq!(c.params.fee_bps, 3);
    assert_eq!(c.run_deposit.0, 4_002_000_000_000_000_000_000);
    assert_eq!(
        t.pool
            .get_markets()
            .iter()
            .map(|m| m.id.clone())
            .collect::<Vec<_>>(),
        vec!["eth-usdc"]
    );

    let bad: Vec<(PoolCall, &str)> = vec![
        (
            Box::new(|p| p.set_project("p".into(), "xyz".into())),
            "version_key",
        ),
        (
            Box::new(|p| p.set_result_key("ed25519:short".into())),
            "ed25519",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    max_sub_batches: 0,
                    ..params()
                })
            }),
            "max_sub_batches",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    max_sub_batches: 13,
                    ..params()
                })
            }),
            "max_sub_batches",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    fee_bps: 10_001,
                    ..params()
                })
            }),
            "fee_bps",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    deadline_margin_s: 0,
                    ..params()
                })
            }),
            "deadline_margin_s",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    deadline_margin_s: 86_401,
                    ..params()
                })
            }),
            "deadline_margin_s",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    max_lots_per_side: 7,
                    ..params()
                })
            }),
            "max_lots_per_side",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    fee_bps: 1_001,
                    ..params()
                })
            }),
            "fee_bps",
        ),
        (
            Box::new(|p| {
                p.set_params(Params {
                    quote_max_age_s: 86_401,
                    ..params()
                })
            }),
            "quote_max_age_s",
        ),
        (
            Box::new(|p| {
                p.add_market(Market {
                    id: "q".into(),
                    lot: U128(u128::MAX),
                    ..market()
                })
            }),
            "lot too large",
        ),
        (
            Box::new(|p| {
                p.add_market(Market {
                    id: "eth-usdc".into(),
                    ..market()
                })
            }),
            "market exists",
        ),
        (
            Box::new(|p| {
                p.add_market(Market {
                    id: "Bad Id".into(),
                    ..market()
                })
            }),
            "market id",
        ),
        (
            Box::new(|p| {
                p.add_market(Market {
                    id: "q".into(),
                    quote: "nep141:wrap.near".into(),
                    ..market()
                })
            }),
            "differ",
        ),
        (
            Box::new(|p| {
                p.add_market(Market {
                    id: "q".into(),
                    base: "has \"quote".into(),
                    ..market()
                })
            }),
            "token id",
        ),
        (
            Box::new(|p| {
                p.add_market(Market {
                    id: "q".into(),
                    lot: U128(0),
                    ..market()
                })
            }),
            "positive",
        ),
        (
            Box::new(|p| p.remove_market("nope".into())),
            "unknown market",
        ),
        (Box::new(|p| p.set_run_deposit(U128(0))), "positive"),
    ];
    for (f, why) in bad {
        env_as(&owner(), 0, 100);
        let p = &mut t.pool;
        assert!(panics(|| f(p)).contains(why), "{why}");
    }
}

#[test]
fn market_cap_holds() {
    let mut t = T::new();
    env_as(&owner(), 0, 100);
    for i in 1..MAX_MARKETS {
        t.pool.add_market(Market {
            id: format!("m{i}"),
            ..market()
        });
    }
    assert!(panics(|| t.pool.add_market(Market {
        id: "one-more".into(),
        ..market()
    }))
    .contains("too many"));
}

// ================================================================ views

#[test]
fn pending_views_paginate_and_hide_ciphertext() {
    let mut t = T::new();
    for i in 0..5u8 {
        let who = if i % 2 == 0 { alice() } else { bob() };
        t.submit_len(&who, SMALL, i);
    }
    let all = t.pool.get_pending(None, None);
    assert_eq!(all.len(), 5);
    assert!(all
        .iter()
        .all(|o| o.sealed_b64.is_none() && o.bytes as usize == SMALL && !o.in_flight));
    assert_eq!(all[1].submitter, bob());
    assert_eq!(all[2].submitted_at, U64(NOW));
    let page = t.pool.get_pending(Some(1), Some(2));
    assert_eq!(page.iter().map(|o| o.id.0).collect::<Vec<_>>(), vec![1, 2]);

    let o = t.pool.get_order(U64(3)).unwrap();
    assert_eq!(o.sealed_b64, Some(B64.encode(vec![3u8; SMALL])));
    let json = sj::to_value(&o).unwrap();
    assert_eq!(json["id"], "3");
    assert_eq!(json["seq"], 3);
    assert!(sj::to_value(&all[0]).unwrap().get("sealed_b64").is_none());
    assert!(t.pool.get_order(U64(99)).is_none());
}

#[test]
fn batch_views_paginate_and_old_records_are_pruned() {
    let mut t = T::new();
    for _ in 0..(BATCH_RETENTION + 3) {
        let (b, ids, _) = t.run();
        t.deliver(b, &ids, signed(&batch_result(&[], &[], json!([]))));
    }
    let next = BATCH_RETENTION + 3;
    assert_eq!(t.pool.get_config().next_batch_id, next);
    assert!(t.pool.get_batch(U64(0)).is_none());
    assert!(t.pool.get_batch(U64(2)).is_none());
    assert!(t.pool.get_batch(U64(3)).is_some());

    let latest = t.pool.get_batches(None, Some(5));
    assert_eq!(
        latest.iter().map(|r| r.id.0).collect::<Vec<_>>(),
        (next - 5..next).collect::<Vec<_>>()
    );
    let from = t.pool.get_batches(Some(U64(1)), Some(4));
    assert_eq!(from.iter().map(|r| r.id.0).collect::<Vec<_>>(), vec![3, 4]);
    assert_eq!(t.pool.get_batches(None, Some(500)).len(), 50);
    let json = sj::to_value(t.pool.get_batch(U64(5)).unwrap()).unwrap();
    assert_eq!(json["status"], "completed");
    assert_eq!(json["op"], "batch");
}

// ================================================================ matcher v1 (§3.4)

#[test]
fn unsigned_error_output_is_a_failed_batch() {
    let mut t = T::new();
    t.submit(&alice());
    let (b, ids, _) = t.run();
    let out = PromiseResult::Successful(br#"{"error":"result_key_unavailable"}"#.to_vec());
    assert_eq!(t.deliver(b, &ids, out), BatchStatus::Failed);
    assert_eq!(
        t.batch(b).error.as_deref(),
        Some("matcher error: result_key_unavailable")
    );
    assert_eq!(t.pending_ids(), vec![0]);
    assert!(transfers().is_empty() && calls().is_empty());
    assert_eq!(t.pool.get_stats().totals.failed, 1);
}

#[test]
fn signed_failed_result_keeps_every_order() {
    let mut t = T::new();
    t.submit(&alice());
    let (b, ids, _) = t.run();
    let h = t.batch(b).input_hash;
    let r = json!({"v": 1, "op": "batch", "error": "book_conflict", "input_sha256": h, "now_ns": "1",
                   "ingested": [], "rejected": [], "deferred": [], "batches": [], "settle": [], "book": null});
    assert_eq!(
        t.deliver(b, &ids, signed(&r.to_string())),
        BatchStatus::Failed
    );
    assert_eq!(
        t.batch(b).error.as_deref(),
        Some("matcher error: book_conflict")
    );
    assert_eq!(t.pending_ids(), vec![0]);
    assert!(transfers().is_empty() && calls().is_empty());
}

#[test]
fn result_for_another_input_is_refused() {
    // A validly signed result of an earlier batch replayed onto a later one.
    let mut t = T::new();
    t.submit(&alice());
    let (b0, ids0, _) = t.run();
    let old = batch_result(&[], &[], json!([]));
    t.deliver(b0, &ids0, signed(&old));
    t.submit(&bob());
    let (b1, ids1, _) = t.run();
    assert_eq!(ids1, vec![0, 1]);
    let replay = old.replace("\"ingested\":[]", "\"ingested\":[\"0\",\"1\"]");
    assert_eq!(
        t.deliver(b1, &ids1, signed(&replay)),
        BatchStatus::InvalidResult
    );
    assert_eq!(
        t.batch(b1).error.as_deref(),
        Some("result answers another input")
    );
    assert_eq!(t.pending_ids(), vec![0, 1]);
}

#[test]
fn deferred_orders_stay_pending_and_are_recorded() {
    let mut t = T::new();
    t.submit(&alice());
    t.submit(&bob());
    let (b, ids, _) = t.run();
    let mut r: Value = sj::from_str(&batch_result(&["0"], &[], json!([]))).unwrap();
    r["deferred"] = json!(["1", "77"]);
    t.deliver(b, &ids, signed(&r.to_string()));
    let rec = t.batch(b);
    assert_eq!(rec.ingested, vec![U64(0)]);
    assert_eq!(rec.deferred, vec![U64(1)]);
    assert_eq!(t.pending_ids(), vec![1]);
    assert_eq!(events("batch_result")[0]["deferred"], json!(["1"]));
}

#[test]
fn stats_follow_each_markets_sub_batches() {
    let mut t = T::new();
    env_as(&owner(), 0, 100);
    t.pool.add_market(Market {
        id: "eth-usdc".into(),
        base: "nep141:eth.omft.near".into(),
        ..market()
    });
    let (b, ids, _) = t.run();
    let mut a = summary();
    a["sub_batches"] = json!(2);
    let mut e = summary();
    e["market"] = json!("eth-usdc");
    e["matched_lots"] = json!(1);
    let mut skipped = summary();
    skipped["market"] = json!("idle");
    skipped["matched_lots"] = json!(0);
    skipped["sub_batches"] = json!(0);
    skipped["skipped"] = json!("no_cross");
    let mut r: Value = sj::from_str(&batch_result(
        &[],
        &[],
        json!([[payload("a1")], [payload("a2")], [payload("e1")]]),
    ))
    .unwrap();
    r["batches"] = json!([a, skipped, e]);
    t.deliver(b, &ids, signed(&r.to_string()));
    assert_eq!(t.batch(b).markets[1].skipped.as_deref(), Some("no_cross"));
    // wnear-usdc's two sub-batches settle, eth-usdc's one fails.
    assert_eq!(
        t.settled(b, vec![ok(), ok(), PromiseResult::Failed]),
        BatchStatus::PartiallySettled
    );
    let s = t.pool.get_stats();
    assert_eq!(s.markets.len(), 1);
    assert_eq!(
        (
            s.markets[0].market.as_str(),
            s.markets[0].batches,
            s.markets[0].crossed_lots
        ),
        ("wnear-usdc", 1, 2)
    );

    // Counts that do not add up fall back to all-or-nothing.
    let (b, ids, _) = t.run();
    t.deliver(
        b,
        &ids,
        signed(&batch_result(
            &[],
            &[],
            json!([[payload("x")], [payload("y")]]),
        )),
    );
    assert_eq!(
        t.settled(b, vec![ok(), PromiseResult::Failed]),
        BatchStatus::PartiallySettled
    );
    assert_eq!(t.pool.get_stats().markets[0].batches, 1);
    let (b, ids, _) = t.run();
    t.deliver(
        b,
        &ids,
        signed(&batch_result(
            &[],
            &[],
            json!([[payload("x")], [payload("y")]]),
        )),
    );
    assert_eq!(t.settled(b, vec![ok(), ok()]), BatchStatus::Settled);
    assert_eq!(t.pool.get_stats().markets[0].batches, 2);
}

#[test]
fn keys_error_result_is_a_failed_run() {
    let mut t = T::new();
    let b = t.run_keys();
    let r = json!({"v": 1, "op": "keys", "error": "key_unavailable"}).to_string();
    assert_eq!(t.deliver(b, &[], signed(&r)), BatchStatus::Failed);
    assert_eq!(
        t.batch(b).error.as_deref(),
        Some("matcher error: key_unavailable")
    );
    assert_eq!(t.pool.get_matcher_key(), None);

    let mut t = T::unpinned();
    let b = t.run_keys();
    assert_eq!(t.deliver(b, &[], signed(&r)), BatchStatus::Failed);
    assert!(t.batch(b).keys.is_none());
}

#[test]
fn switching_project_clears_keys_and_rebootstraps() {
    let mut t = T::new();
    let b = t.run_keys();
    t.deliver(
        b,
        &[],
        signed(&keys_result(X25519, VENUE, &key_str(&result_sk()))),
    );
    assert!(t.pool.get_matcher_key().is_some());

    // A new version of the same project keeps the keys.
    env_as(&owner(), 0, t.height);
    t.pool
        .set_project("blindbook.near/matcher".into(), "c".repeat(64));
    let c = t.pool.get_config();
    assert!(c.result_key.is_some() && c.matcher_key.is_some() && c.venue_key.is_some());

    // Another project does not: its host keys differ.
    env_as(&owner(), 0, t.height);
    t.pool
        .set_project("blindbook.near/matcher-v2".into(), VERSION_KEY.into());
    let c = t.pool.get_config();
    assert_eq!(
        (c.result_key, c.matcher_key, c.venue_key),
        (None, None, None)
    );
    let changed = events("config_changed");
    assert_eq!(changed[0]["field"], "keys_cleared");
    assert_eq!(changed[1]["field"], "project");

    let b = t.run_keys();
    let claimed = keys_result(X25519, VENUE, &key_str(&other_sk()));
    assert_eq!(
        t.deliver(
            b,
            &[],
            sign_with(&other_sk(), &key_str(&other_sk()), &claimed)
        ),
        BatchStatus::Unpinned
    );
    assert_eq!(t.batch(b).keys.unwrap().result_key, key_str(&other_sk()));
}

#[test]
fn execute_intents_args_are_byte_exact() {
    use serde_json::value::RawValue;
    let raw: Vec<Box<RawValue>> = vec![
        RawValue::from_string(r#"{"b":1, "a" : [2]}"#.into()).unwrap(),
        RawValue::from_string(r#""x""#.into()).unwrap(),
    ];
    let refs: Vec<&RawValue> = raw.iter().map(|r| r.as_ref()).collect();
    assert_eq!(
        String::from_utf8(execute_intents_args(&refs)).unwrap(),
        r#"{"signed":[{"b":1, "a" : [2]},"x"]}"#
    );
}

#[test]
fn key_and_text_helpers() {
    let k = key_str(&result_sk());
    assert_eq!(normalize_ed25519_key(&k), Some(k.clone()));
    assert_eq!(parse_ed25519_key("ed25519:"), None);
    assert_eq!(
        parse_ed25519_key(&k.replace("ed25519:", "secp256k1:")),
        None
    );
    assert!(is_x25519_hex(X25519));
    assert!(!is_x25519_hex(&X25519[1..]));
    assert_eq!(clip(&"é".repeat(100)).len(), 128);
    assert_eq!(floor_char_boundary("aé", 2), 1);
}
