//! Test harness: a mock OutLayer host, a mock NEAR Intents verifier that
//! checks what the real one checks (signatures, keys, versioned nonces, salts,
//! deadlines, live balances and the per-token fee invariant), real 1Click
//! quote fixtures, and traders that sign real NEP-413 and ERC-191 lots.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use blindbook_core::envelope::{seal, MatcherKey};
use blindbook_core::intent::{nep413_hash, parse_rfc3339_ns, recover_erc191};
use blindbook_core::settle::protocol_fee;
use blindbook_matcher::nonce;
use blindbook_matcher::oneclick::QUOTE_URL;
use blindbook_matcher::rpc::{self, TESTNET_RPC};
use blindbook_matcher::util::{ed25519_bytes, ed25519_str, hex, rfc3339, sha256, NS_PER_S};
use blindbook_matcher::{Host, HostError, HttpRequest, HttpResponse, KeyId, Nep413Signed};
use ed25519_dalek::{Signer as _, SigningKey, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};

pub const VERIFIER: &str = "intents-bbcced2c.testnet";
pub const VENUE: &str = "venue.blindbook-dev.testnet";
pub const MARKET: &str = "tnear-tusdc";
pub const BASE: &str = "nep141:tnear-bbcced2c.testnet";
pub const QUOTE: &str = "nep141:tusdc-bbcced2c.testnet";
pub const REF_BASE: &str = "nep141:wrap.near";
pub const REF_QUOTE: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
/// 0.1 NEAR.
pub const LOT: u128 = 100_000_000_000_000_000_000_000;
pub const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
pub const PROBE_BASE: u128 = 10 * NEAR;
pub const PROBE_QUOTE: u128 = 36_000_000;
pub const SALT: &str = "0d9327b2";
/// 2026-11-09T12:29:30Z, ~20 s after the 1Click fixtures were signed.
pub const NOW_S: u64 = 1_794_227_370;
pub const NOW: u64 = NOW_S * NS_PER_S;
/// Lots of test orders expire an hour later.
pub const DEADLINE_S: u64 = NOW_S + 3_600;
/// A seller limit below and a buyer limit above the fixture midpoint
/// (~353 872 µUSDC per 0.1 NEAR).
pub const SELL_LIMIT: u128 = 350_000;
pub const BUY_LIMIT: u128 = 358_000;

pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../crates/core/tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(path).unwrap()
}

// ---------------------------------------------------------------- verifier

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimMode {
    /// Execute like the verifier.
    Real,
    /// Panic on every simulation.
    Panic,
    /// Report an invariant violation on every multi-payload simulation.
    Invariant,
}

#[derive(Debug, Clone)]
pub struct MockVerifier {
    pub id: String,
    pub salt: String,
    pub valid_salts: BTreeSet<String>,
    pub fee: u32,
    pub used: BTreeSet<(String, String)>,
    pub keys: BTreeSet<(String, String)>,
    pub balances: BTreeMap<(String, String), u128>,
    /// Views that fail at the transport level.
    pub down: BTreeSet<String>,
    pub sim: SimMode,
    /// Signers whose payloads make the simulation panic (a revoked key, say).
    pub poisoned: BTreeSet<String>,
    pub now_ns: u64,
}

impl MockVerifier {
    fn new() -> Self {
        Self {
            id: VERIFIER.into(),
            salt: SALT.into(),
            valid_salts: [SALT.to_string()].into(),
            fee: 1,
            used: BTreeSet::new(),
            keys: BTreeSet::new(),
            balances: BTreeMap::new(),
            down: BTreeSet::new(),
            sim: SimMode::Real,
            poisoned: BTreeSet::new(),
            now_ns: NOW,
        }
    }

    pub fn balance(&self, account: &str, token: &str) -> u128 {
        self.balances
            .get(&(account.to_string(), token.to_string()))
            .copied()
            .unwrap_or(0)
    }

    fn view(&self, method: &str, args: &Value) -> Result<Value, String> {
        let s = |k: &str| args[k].as_str().unwrap_or_default().to_string();
        match method {
            "current_salt" => Ok(json!(self.salt)),
            "fee" => Ok(json!(self.fee)),
            "is_valid_salt" => Ok(json!(self.valid_salts.contains(&s("salt")))),
            "is_nonce_used" => Ok(json!(self.used.contains(&(s("account_id"), s("nonce"))))),
            "has_public_key" => Ok(json!(self.keys.contains(&(s("account_id"), s("public_key"))))),
            "mt_balance_of" => Ok(json!(self.balance(&s("account_id"), &s("token_id")).to_string())),
            "simulate_intents" => self.simulate(args["signed"].as_array().ok_or("signed")?),
            other => Err(format!("MethodNotFound {other}")),
        }
    }

    /// Apply a batch to a copy of the state, as `execute_intents` would.
    pub fn simulate(&self, signed: &[Value]) -> Result<Value, String> {
        if self.sim == SimMode::Panic {
            return Err("Smart contract panicked: simulated failure".into());
        }
        let mut bal = self.balances.clone();
        let mut used = self.used.clone();
        let mut net: BTreeMap<String, i128> = BTreeMap::new();
        let mut executed = Vec::new();
        for p in signed {
            let (signer, inner, nonce_b64) = self.authenticate(p)?;
            if self.poisoned.contains(&signer) {
                return Err(format!("public key doesn't exist for account '{signer}'"));
            }
            let deadline = parse_rfc3339_ns(inner["deadline"].as_str().ok_or("deadline")?).ok_or("deadline")?;
            if deadline <= self.now_ns {
                return Err("deadline has expired".into());
            }
            let raw: [u8; 32] = base64::engine::general_purpose::STANDARD
                .decode(&nonce_b64)
                .map_err(|_| "nonce")?
                .try_into()
                .map_err(|_| "nonce")?;
            let v = nonce::decode(&raw).ok_or("legacy nonce")?;
            if !self.valid_salts.contains(&hex(&v.salt)) {
                return Err("invalid salt".into());
            }
            if (v.deadline_ns as u64) < deadline {
                return Err("deadline is greater than nonce".into());
            }
            if !used.insert((signer.clone(), nonce_b64.clone())) {
                return Err("nonce was already used".into());
            }
            for intent in inner["intents"].as_array().ok_or("intents")? {
                match intent["intent"].as_str() {
                    Some("token_diff") => {
                        for (token, d) in intent["diff"].as_object().ok_or("diff")? {
                            let d: i128 = d.as_str().ok_or("amount")?.parse().map_err(|_| "amount")?;
                            move_balance(&mut bal, &signer, token, d)?;
                            *net.entry(token.clone()).or_default() += d + protocol_fee(token, d, self.fee);
                        }
                    }
                    Some("transfer") => {
                        let to = intent["receiver_id"].as_str().ok_or("receiver")?;
                        for (token, a) in intent["tokens"].as_object().ok_or("tokens")? {
                            let a: i128 = a.as_str().ok_or("amount")?.parse().map_err(|_| "amount")?;
                            move_balance(&mut bal, &signer, token, -a)?;
                            move_balance(&mut bal, to, token, a)?;
                        }
                    }
                    _ => return Err("unsupported intent".into()),
                }
            }
            executed.push(json!({ "account_id": signer, "nonce": nonce_b64 }));
        }
        net.retain(|_, v| *v != 0);
        let mut out = json!({ "intents_executed": executed, "logs": [], "min_deadline": "0", "state": { "fee": self.fee, "current_salt": self.salt } });
        if !net.is_empty() || (self.sim == SimMode::Invariant && signed.len() > 1) {
            let deltas: BTreeMap<String, String> = net.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
            out["invariant_violated"] = json!({ "error": "unmatched_deltas", "unmatched_deltas": deltas });
        }
        Ok(out)
    }

    /// Check a signed payload the way the verifier does; returns the signer,
    /// the signed message and the nonce.
    fn authenticate(&self, p: &Value) -> Result<(String, Value, String), String> {
        match p["standard"].as_str() {
            Some("nep413") => {
                let payload = &p["payload"];
                let message = payload["message"].as_str().ok_or("message")?;
                let nonce_b64 = payload["nonce"].as_str().ok_or("nonce")?;
                let recipient = payload["recipient"].as_str().ok_or("recipient")?;
                if recipient != self.id {
                    return Err("wrong verifying contract".into());
                }
                let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
                    .decode(nonce_b64)
                    .map_err(|_| "nonce")?
                    .try_into()
                    .map_err(|_| "nonce")?;
                let pk_str = p["public_key"].as_str().ok_or("public_key")?;
                let pk: [u8; 32] = ed25519_bytes(pk_str).ok_or("pk")?.try_into().map_err(|_| "pk")?;
                let sig: [u8; 64] = ed25519_bytes(p["signature"].as_str().ok_or("sig")?)
                    .ok_or("sig")?
                    .try_into()
                    .map_err(|_| "sig")?;
                let key = VerifyingKey::from_bytes(&pk).map_err(|_| "pk")?;
                key.verify(
                    &nep413_hash(message, &nonce, recipient, None),
                    &ed25519_dalek::Signature::from_bytes(&sig),
                )
                .map_err(|_| "invalid signature")?;
                let inner: Value = serde_json::from_str(message).map_err(|_| "message json")?;
                let signer = inner["signer_id"].as_str().ok_or("signer")?.to_string();
                let implicit = hex(&pk) == signer;
                if !implicit && !self.keys.contains(&(signer.clone(), pk_str.to_string())) {
                    return Err(format!("public key '{pk_str}' doesn't exist for account '{signer}'"));
                }
                Ok((signer, inner, nonce_b64.to_string()))
            }
            Some("erc191") => {
                let payload = p["payload"].as_str().ok_or("payload")?;
                let address =
                    recover_erc191(payload, p["signature"].as_str().ok_or("sig")?).map_err(|e| e.to_string())?;
                let inner: Value = serde_json::from_str(payload).map_err(|_| "payload json")?;
                if inner["signer_id"] != address || inner["verifying_contract"] != self.id.as_str() {
                    return Err("invalid signature".into());
                }
                let nonce = inner["nonce"].as_str().ok_or("nonce")?.to_string();
                Ok((address, inner, nonce))
            }
            _ => Err("unknown standard".into()),
        }
    }
}

fn move_balance(
    bal: &mut BTreeMap<(String, String), u128>,
    account: &str,
    token: &str,
    delta: i128,
) -> Result<(), String> {
    let b = bal.entry((account.to_string(), token.to_string())).or_default();
    if delta < 0 {
        *b = b
            .checked_sub(delta.unsigned_abs())
            .ok_or("insufficient balance or overflow")?;
    } else {
        *b += delta as u128;
    }
    Ok(())
}

// ---------------------------------------------------------------- host

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteMode {
    Fixture,
    Status(u16),
    Tampered,
    Down,
}

pub struct MockHost {
    pub storage: BTreeMap<String, Vec<u8>>,
    pub storage_down: bool,
    pub cas_conflict: bool,
    pub keys_down: bool,
    pub x25519_seed: [u8; 32],
    pub venue: SigningKey,
    pub result: SigningKey,
    pub now: u64,
    pub rng: u8,
    pub verifier: MockVerifier,
    pub quotes: QuoteMode,
    /// Every request made, as `method` (RPC) or `quote`.
    pub calls: Vec<String>,
    /// Arguments of every RPC call, for assertions.
    pub call_args: Vec<(String, Value)>,
}

impl Default for MockHost {
    fn default() -> Self {
        Self {
            storage: BTreeMap::new(),
            storage_down: false,
            cas_conflict: false,
            keys_down: false,
            x25519_seed: [0x11; 32],
            venue: SigningKey::from_bytes(&[0x22; 32]),
            result: SigningKey::from_bytes(&[0x33; 32]),
            now: NOW,
            rng: 0,
            verifier: MockVerifier::new(),
            quotes: QuoteMode::Fixture,
            calls: vec![],
            call_args: vec![],
        }
    }
}

impl MockHost {
    pub fn matcher_key(&self) -> MatcherKey {
        MatcherKey::from_seed(self.x25519_seed)
    }

    pub fn venue_pk(&self) -> String {
        ed25519_str(&self.venue.verifying_key().to_bytes())
    }

    pub fn result_vk(&self) -> VerifyingKey {
        self.result.verifying_key()
    }

    pub fn count(&self, call: &str) -> usize {
        self.calls.iter().filter(|c| *c == call).count()
    }

    fn quote(&self, req: &HttpRequest) -> Result<HttpResponse, HostError> {
        let body: Value = serde_json::from_slice(&req.body).map_err(|_| HostError::new("bad quote request"))?;
        let name = if body["originAsset"] == REF_BASE {
            "quote_sell"
        } else {
            "quote_buy"
        };
        match self.quotes {
            QuoteMode::Fixture => Ok(HttpResponse {
                status: 201,
                body: fixture(name),
            }),
            QuoteMode::Status(s) => Ok(HttpResponse {
                status: s,
                body: br#"{"message":"error"}"#.to_vec(),
            }),
            QuoteMode::Tampered => {
                let mut v: Value = serde_json::from_slice(&fixture(name)).unwrap();
                let out: u128 = v["quote"]["amountOut"].as_str().unwrap().parse().unwrap();
                v["quote"]["amountOut"] = json!((out * 2).to_string());
                Ok(HttpResponse {
                    status: 201,
                    body: v.to_string().into_bytes(),
                })
            }
            QuoteMode::Down => Err(HostError::new("connect timeout")),
        }
    }
}

impl Host for MockHost {
    fn storage_get(&mut self, key: &str) -> Result<Option<Vec<u8>>, HostError> {
        if self.storage_down {
            return Err(HostError::new("storage down"));
        }
        Ok(self.storage.get(key).cloned())
    }

    fn storage_put(&mut self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<bool, HostError> {
        if self.storage_down {
            return Err(HostError::new("storage down"));
        }
        if self.cas_conflict || self.storage.get(key).map(Vec::as_slice) != expected {
            return Ok(false);
        }
        self.storage.insert(key.to_string(), value.to_vec());
        Ok(true)
    }

    fn x25519_seed(&mut self) -> Result<[u8; 32], HostError> {
        if self.keys_down {
            return Err(HostError::new("keystore down"));
        }
        Ok(self.x25519_seed)
    }

    fn public_key(&mut self, key: KeyId) -> Result<[u8; 32], HostError> {
        if self.keys_down {
            return Err(HostError::new("keystore down"));
        }
        Ok(match key {
            KeyId::Venue => self.venue.verifying_key().to_bytes(),
            KeyId::Result => self.result.verifying_key().to_bytes(),
        })
    }

    fn sign_result(&mut self, digest: &[u8; 32]) -> Result<[u8; 64], HostError> {
        if self.keys_down {
            return Err(HostError::new("keystore down"));
        }
        Ok(self.result.sign(digest).to_bytes())
    }

    fn sign_venue_nep413(
        &mut self,
        message: &str,
        recipient: &str,
        nonce: &[u8; 32],
    ) -> Result<Nep413Signed, HostError> {
        if self.keys_down {
            return Err(HostError::new("keystore down"));
        }
        let hash = nep413_hash(message, nonce, recipient, None);
        Ok(Nep413Signed {
            public_key: self.venue.verifying_key().to_bytes(),
            signature: self.venue.sign(&hash).to_bytes(),
        })
    }

    fn http_post(&mut self, requests: &[HttpRequest]) -> Vec<Result<HttpResponse, HostError>> {
        requests
            .iter()
            .map(|r| {
                if r.url == QUOTE_URL {
                    self.calls.push("quote".into());
                    return self.quote(r);
                }
                assert_eq!(r.url, TESTNET_RPC, "testnet build must use the testnet RPC");
                let (contract, method, args) = rpc::decode_request(r).expect("well-formed RPC request");
                self.calls.push(method.clone());
                self.call_args.push((method.clone(), args.clone()));
                if contract != self.verifier.id {
                    return Ok(HttpResponse {
                        status: 200,
                        body: rpc::encode_panic("CodeDoesNotExist"),
                    });
                }
                if self.verifier.down.contains(&method) {
                    return Err(HostError::new("connection reset"));
                }
                let body = match self.verifier.view(&method, &args) {
                    Ok(v) => rpc::encode_result(&v),
                    Err(msg) => rpc::encode_panic(&msg),
                };
                Ok(HttpResponse { status: 200, body })
            })
            .collect()
    }

    fn now_ns(&mut self) -> u64 {
        self.now
    }

    fn random(&mut self, out: &mut [u8]) -> Result<(), HostError> {
        for b in out.iter_mut() {
            self.rng = self.rng.wrapping_add(1);
            *b = self.rng;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- traders

pub enum Signer {
    Nep413(SigningKey),
    Evm(k256::ecdsa::SigningKey),
}

pub struct Trader {
    pub account: String,
    pub signer: Signer,
}

impl Trader {
    /// A named NEAR account; its key must be registered on the verifier.
    pub fn named(account: &str, seed: u8) -> Self {
        Self {
            account: account.into(),
            signer: Signer::Nep413(SigningKey::from_bytes(&[seed; 32])),
        }
    }

    /// An implicit NEAR account signing with its own key.
    pub fn implicit(seed: u8) -> Self {
        let key = SigningKey::from_bytes(&[seed; 32]);
        Self {
            account: hex(&key.verifying_key().to_bytes()),
            signer: Signer::Nep413(key),
        }
    }

    /// An EVM wallet: its lowercase address is the verifier account.
    pub fn evm(seed: u8) -> Self {
        let sk = k256::ecdsa::SigningKey::from_slice(&[seed; 32]).unwrap();
        let point = sk.verifying_key().to_encoded_point(false);
        let addr = format!("0x{}", hex(&Keccak256::digest(&point.as_bytes()[1..])[12..]));
        Self {
            account: addr,
            signer: Signer::Evm(sk),
        }
    }

    pub fn public_key(&self) -> Option<String> {
        match &self.signer {
            Signer::Nep413(k) => Some(ed25519_str(&k.verifying_key().to_bytes())),
            Signer::Evm(_) => None,
        }
    }

    /// One signed lot: exactly one `token_diff` for one lot at `limit`.
    pub fn lot(&self, side: &str, limit: u128, deadline_s: u64, random: u8) -> Value {
        let diff = if side == "sell" {
            json!({ BASE: format!("-{LOT}"), QUOTE: limit.to_string() })
        } else {
            json!({ BASE: LOT.to_string(), QUOTE: format!("-{limit}") })
        };
        self.sign(
            json!([{ "intent": "token_diff", "diff": diff }]),
            deadline_s,
            random,
            SALT,
        )
    }

    /// Sign arbitrary intents with a versioned nonce.
    pub fn sign(&self, intents: Value, deadline_s: u64, random: u8, salt: &str) -> Value {
        let salt: [u8; 4] = blindbook_matcher::util::unhex(salt).unwrap().try_into().unwrap();
        let nonce = nonce::encode(salt, (deadline_s * NS_PER_S) as i64, [random; 15]);
        let nonce_b64 = base64::engine::general_purpose::STANDARD.encode(nonce);
        let deadline = rfc3339(deadline_s);
        match &self.signer {
            Signer::Nep413(key) => {
                let message =
                    json!({ "signer_id": self.account, "deadline": deadline, "intents": intents }).to_string();
                let sig = key.sign(&nep413_hash(&message, &nonce, VERIFIER, None));
                json!({
                    "standard": "nep413",
                    "payload": { "message": message, "nonce": nonce_b64, "recipient": VERIFIER },
                    "public_key": ed25519_str(&key.verifying_key().to_bytes()),
                    "signature": ed25519_str(&sig.to_bytes()),
                })
            }
            Signer::Evm(sk) => {
                let payload = json!({
                    "signer_id": self.account,
                    "verifying_contract": VERIFIER,
                    "deadline": deadline,
                    "nonce": nonce_b64,
                    "intents": intents,
                })
                .to_string();
                let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", payload.len(), payload);
                let (sig, recid) = sk
                    .sign_prehash_recoverable(&Keccak256::digest(prefixed.as_bytes()))
                    .unwrap();
                let mut raw = sig.to_bytes().to_vec();
                raw.push(recid.to_byte());
                json!({ "standard": "erc191", "payload": payload, "signature": format!("secp256k1:{}", bs58::encode(raw).into_string()) })
            }
        }
    }

    /// The nonce of a signed lot.
    pub fn nonce_of(lot: &Value) -> String {
        match lot["standard"].as_str() {
            Some("nep413") => lot["payload"]["nonce"].as_str().unwrap().to_string(),
            _ => serde_json::from_str::<Value>(lot["payload"].as_str().unwrap()).unwrap()["nonce"]
                .as_str()
                .unwrap()
                .to_string(),
        }
    }
}

// ---------------------------------------------------------------- harness

pub struct Order {
    pub id: String,
    pub body: Value,
    pub sealed: String,
    pub seq: u64,
}

impl Order {
    pub fn input(&self) -> Value {
        json!({ "id": self.id, "seq": self.seq, "market": MARKET, "sealed": self.sealed })
    }

    pub fn nonces(&self) -> Vec<String> {
        self.body["lots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| Trader::nonce_of(&l["intent"]))
            .collect()
    }
}

pub struct Harness {
    pub host: MockHost,
    pub next_id: u64,
    pub params: Value,
    /// Envelope randomness counter.
    pub eph: u8,
}

impl Default for Harness {
    fn default() -> Self {
        let mut host = MockHost::default();
        // the venue holds dust of both tokens
        host.verifier.balances.insert((VENUE.into(), BASE.into()), NEAR);
        host.verifier.balances.insert((VENUE.into(), QUOTE.into()), 10_000_000);
        host.verifier.keys.insert((VENUE.into(), host.venue_pk()));
        Self {
            host,
            next_id: 1,
            params: json!({
                "fee_bps": 5,
                "max_spread_bps": 150,
                "max_lots_per_side": 6,
                "max_sub_batches": 4,
                "deadline_margin_s": 90,
            }),
            eph: 0,
        }
    }
}

pub struct Run {
    pub out: Value,
    pub result: Value,
    pub raw: String,
}

impl Harness {
    /// Fund a trader and, for a named NEAR account, register its key.
    pub fn fund(&mut self, t: &Trader, token: &str, amount: u128) {
        *self
            .host
            .verifier
            .balances
            .entry((t.account.clone(), token.to_string()))
            .or_default() += amount;
        if let Some(pk) = t.public_key() {
            if hex(&ed25519_bytes(&pk).unwrap()) != t.account {
                self.host.verifier.keys.insert((t.account.clone(), pk));
            }
        }
    }

    /// Fund a trader generously for either side.
    pub fn fund_both(&mut self, t: &Trader) {
        self.fund(t, BASE, 1_000 * NEAR);
        self.fund(t, QUOTE, 1_000_000_000_000);
    }

    pub fn body(&self, t: &Trader, side: &str, limits: &[u128], min_lots: u16, random: u8) -> Value {
        let lots: Vec<Value> = limits
            .iter()
            .enumerate()
            .map(|(i, l)| {
                json!({ "index": i, "limit_quote": l.to_string(), "intent": t.lot(side, *l, DEADLINE_S, random.wrapping_add(i as u8)) })
            })
            .collect();
        json!({ "v": 1, "market": MARKET, "owner": t.account, "side": side, "min_lots": min_lots, "lots": lots })
    }

    pub fn seal_body(&mut self, body: &Value) -> String {
        self.eph = self.eph.wrapping_add(1);
        let sealed = seal(
            self.host.matcher_key().public_bytes(),
            body.to_string().as_bytes(),
            [self.eph; 32],
            [self.eph; 24],
        )
        .unwrap();
        base64::engine::general_purpose::STANDARD.encode(sealed)
    }

    pub fn order_from_body(&mut self, body: Value) -> Order {
        let id = self.next_id;
        self.next_id += 1;
        let sealed = self.seal_body(&body);
        Order {
            id: id.to_string(),
            body,
            sealed,
            seq: id,
        }
    }

    /// A sealed order with one lot per limit. `random` seeds the lots' nonces.
    pub fn order(&mut self, t: &Trader, side: &str, limits: &[u128], min_lots: u16, random: u8) -> Order {
        let body = self.body(t, side, limits, min_lots, random);
        self.order_from_body(body)
    }

    pub fn input(&self, orders: &[Value]) -> Value {
        json!({
            "v": 1,
            "op": "batch",
            "network": "testnet",
            "verifier": VERIFIER,
            "venue_account": VENUE,
            "now_ns": NOW.to_string(),
            "markets": [{
                "id": MARKET,
                "base": BASE,
                "quote": QUOTE,
                "lot": LOT.to_string(),
                "reference": { "base": REF_BASE, "quote": REF_QUOTE, "probe_base": PROBE_BASE.to_string(), "probe_quote": PROBE_QUOTE.to_string() },
            }],
            "params": self.params,
            "orders": orders,
        })
    }

    pub fn run_raw(&mut self, input: &[u8]) -> Run {
        self.host.calls.clear();
        self.host.call_args.clear();
        let raw = blindbook_matcher::run(&mut self.host, input);
        let out: Value = serde_json::from_str(&raw).expect("output is JSON");
        let result = verify_output(&out, &self.host.result_vk());
        Run { out, result, raw }
    }

    pub fn run(&mut self, orders: &[&Order]) -> Run {
        let input = self.input(&orders.iter().map(|o| o.input()).collect::<Vec<_>>());
        self.run_raw(input.to_string().as_bytes())
    }

    pub fn book(&self) -> Value {
        serde_json::from_slice(self.host.storage.get("book").expect("book stored")).unwrap()
    }

    /// Execute every settle array against the mock verifier, as the pool
    /// would with `execute_intents`, and commit the state.
    pub fn settle(&mut self, result: &Value) {
        for sub in result["settle"].as_array().unwrap() {
            let signed = sub.as_array().unwrap();
            let sim = self.host.verifier.simulate(signed).expect("settles");
            assert!(sim.get("invariant_violated").is_none(), "sub-batch nets to zero");
            self.commit(signed);
        }
    }

    fn commit(&mut self, signed: &[Value]) {
        // replay the simulation on the live state
        let v = &mut self.host.verifier;
        for p in signed {
            let (signer, inner, nonce) = v.authenticate(p).unwrap();
            v.used.insert((signer.clone(), nonce));
            for intent in inner["intents"].as_array().unwrap() {
                if intent["intent"] == "token_diff" {
                    for (token, d) in intent["diff"].as_object().unwrap() {
                        let d: i128 = d.as_str().unwrap().parse().unwrap();
                        move_balance(&mut v.balances, &signer, token, d).unwrap();
                    }
                } else {
                    let to = intent["receiver_id"].as_str().unwrap();
                    for (token, a) in intent["tokens"].as_object().unwrap() {
                        let a: i128 = a.as_str().unwrap().parse().unwrap();
                        move_balance(&mut v.balances, &signer, token, -a).unwrap();
                        move_balance(&mut v.balances, to, token, a).unwrap();
                    }
                }
            }
        }
    }
}

/// Check the output's signature against the result key and decode `result`.
pub fn verify_output(out: &Value, key: &VerifyingKey) -> Value {
    let result = out["result"]
        .as_str()
        .unwrap_or_else(|| panic!("output has a result: {out}"));
    assert_eq!(out["key"], ed25519_str(&key.to_bytes()), "output names the result key");
    let sig: [u8; 64] = ed25519_bytes(out["sig"].as_str().unwrap()).unwrap().try_into().unwrap();
    key.verify(&sha256(result.as_bytes()), &ed25519_dalek::Signature::from_bytes(&sig))
        .expect("sig is the result key's signature over sha256(result)");
    serde_json::from_str(result).unwrap()
}

pub fn rejected_reason(result: &Value, id: &str) -> Option<String> {
    result["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .map(|r| r["reason"].as_str().unwrap().to_string())
}

pub fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}
