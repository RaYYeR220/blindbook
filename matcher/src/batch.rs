//! `op: "batch"`: reconcile, ingest, price, cross, plan, sign, simulate.
//!
//! Every remote fact the batch needs is fetched in one concurrent wave (views
//! and quotes), the sub-batches it would settle in a second (simulations), and
//! only when a simulation fails, a third (each lot of a failed sub-batch on
//! its own, to find the order at fault). A fact that could not be fetched is
//! unknown, and an unknown fact never leads to settlement: the lot or order
//! waits for the next batch.

use std::collections::{BTreeMap, BTreeSet};

use blindbook_core::envelope::MatcherKey;
use blindbook_core::settle::{plan, SubBatch};
use blindbook_core::{cross, Lot, Market, MatchedLot, Order, Price, Side, SkipReason};
use serde::Serialize;
use serde_json::{json, Value};

use crate::book::{Book, InFlight, BOOK_KEY, MAX_BOOK_BYTES, MAX_LOTS, MAX_ORDERS, MAX_STRIKES};
use crate::host::{Host, HostError, HttpRequest, HttpResponse};
use crate::ingest::{self, lot_salt, salt_bytes, Candidate, Reject};
use crate::input::BatchInput;
use crate::nonce;
use crate::oneclick::{self, QuoteFail};
use crate::rpc::{self, View, ViewError};
use crate::util::{b64, ed25519_str, rfc3339, NS_PER_S};

/// Lifetime of the venue payload: the sub-batch must settle within it.
pub const VENUE_TTL_S: u64 = 180;
/// After a sub-batch's venue deadline, wait this long before treating its
/// unused lots as unsettled, so a lagging RPC node cannot hide a fill.
pub const SETTLE_GUARD_NS: u64 = 30 * NS_PER_S;

// ---------------------------------------------------------------- result

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejected {
    pub id: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BookCounts {
    pub orders: usize,
    pub lots: usize,
}

/// One market's batch, as the public result reports it. Volumes cover only
/// the sub-batches in `settle`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketBatch {
    pub market: String,
    pub mid: Option<Price>,
    pub lot_mid_quote: String,
    pub matched_lots: u32,
    pub base_volume: String,
    pub quote_volume: String,
    pub rebated: String,
    pub venue_fee: String,
    /// The two 1Click signed digests (bid, ask) the midpoint comes from.
    pub quotes: Vec<String>,
    /// Sub-batches of this market in `settle`.
    pub sub_batches: u32,
    /// One reason per sub-batch that crossed but was not included.
    pub dropped: Vec<&'static str>,
    pub skipped: Option<&'static str>,
}

impl MarketBatch {
    fn new(market: &str) -> Self {
        Self {
            market: market.to_string(),
            mid: None,
            lot_mid_quote: "0".into(),
            matched_lots: 0,
            base_volume: "0".into(),
            quote_volume: "0".into(),
            rebated: "0".into(),
            venue_fee: "0".into(),
            quotes: vec![],
            sub_batches: 0,
            dropped: vec![],
            skipped: None,
        }
    }
}

/// The `result` of a batch (PROTOCOL.md §3.2). Field order is the canonical
/// serialization order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchResult {
    pub v: u32,
    pub op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
    /// Hex sha256 of the exact `input_data` bytes this result answers.
    pub input_sha256: String,
    /// Batch time the matcher used, Unix ns.
    pub now_ns: String,
    pub ingested: Vec<String>,
    pub rejected: Vec<Rejected>,
    /// Orders neither ingested nor rejected: still pending at the pool.
    pub deferred: Vec<String>,
    pub batches: Vec<MarketBatch>,
    pub settle: Vec<Vec<Value>>,
    pub book: Option<BookCounts>,
}

impl BatchResult {
    /// The fail-closed answer: nothing ingested, rejected or settled.
    pub fn failed(code: &'static str, input_sha256: String, now_ns: u64) -> Self {
        Self {
            v: 1,
            op: "batch",
            error: Some(code),
            input_sha256,
            now_ns: now_ns.to_string(),
            ingested: vec![],
            rejected: vec![],
            deferred: vec![],
            batches: vec![],
            settle: vec![],
            book: None,
        }
    }
}

pub struct Outcome {
    pub result: BatchResult,
    /// The new book, to store after the result is signed.
    pub book_bytes: Vec<u8>,
    /// The book as read, for the compare-and-swap write.
    pub stored: Option<Vec<u8>>,
}

// ---------------------------------------------------------------- plumbing

#[derive(Default)]
struct Wave {
    requests: Vec<HttpRequest>,
}

impl Wave {
    fn push(&mut self, r: HttpRequest) -> usize {
        self.requests.push(r);
        self.requests.len() - 1
    }

    fn send<H: Host>(self, host: &mut H) -> Result<Vec<Result<HttpResponse, HostError>>, &'static str> {
        if self.requests.is_empty() {
            return Ok(vec![]);
        }
        let out = host.http_post(&self.requests);
        if out.len() != self.requests.len() {
            return Err("host_error");
        }
        Ok(out)
    }
}

enum Fate {
    Ingested,
    Rejected(Reject),
    Deferred,
    Pending(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sim {
    Pass,
    Failed,
    Panicked,
    Unavailable,
}

fn classify_simulation(resp: &Result<HttpResponse, HostError>, signed: usize) -> Sim {
    match rpc::parse(resp) {
        Ok(v) => {
            let clean = v.get("invariant_violated").is_none_or(Value::is_null);
            let executed = v.get("intents_executed").and_then(Value::as_array).map(Vec::len);
            if clean && executed == Some(signed) {
                Sim::Pass
            } else {
                Sim::Failed
            }
        }
        Err(ViewError::Contract(_)) => Sim::Panicked,
        Err(_) => Sim::Unavailable,
    }
}

fn opt<T>(r: Result<T, ViewError>) -> Option<T> {
    r.ok()
}

/// A sub-batch ready to simulate.
struct Planned {
    market: usize,
    sub: SubBatch,
    sells: Vec<MatchedLot>,
    buys: Vec<MatchedLot>,
    lot_mid: u128,
    lot: u128,
    signed: Vec<Value>,
}

fn venue_payload<H: Host>(
    host: &mut H,
    venue: &str,
    verifier: &str,
    salt: [u8; 4],
    deadline_s: u64,
    intents: Vec<Value>,
) -> Result<Value, &'static str> {
    let mut random = [0u8; 15];
    host.random(&mut random).map_err(|_| "rng_unavailable")?;
    let deadline_ns = deadline_s
        .checked_mul(NS_PER_S)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or("clock_invalid")?;
    let nonce = nonce::encode(salt, deadline_ns as i64, random);
    let message = json!({ "signer_id": venue, "deadline": rfc3339(deadline_s), "intents": intents }).to_string();
    let s = host
        .sign_venue_nep413(&message, verifier, &nonce)
        .map_err(|_| "key_unavailable")?;
    Ok(json!({
        "standard": "nep413",
        "payload": { "message": message, "nonce": b64(&nonce), "recipient": verifier },
        "public_key": ed25519_str(&s.public_key),
        "signature": ed25519_str(&s.signature),
    }))
}

fn quote_skip(f: QuoteFail) -> &'static str {
    match f {
        QuoteFail::Unavailable => "quote_unavailable",
        QuoteFail::Unverified => "quote_unverified",
        QuoteFail::Stale => "quote_stale",
    }
}

// ---------------------------------------------------------------- the batch

pub fn run<H: Host>(host: &mut H, input: &BatchInput, input_sha256: String) -> Result<Outcome, &'static str> {
    let rpc_url = rpc::endpoint(&input.network).ok_or("bad_input")?;
    let verifier = input.verifier.as_str();
    let params = &input.params;
    let now = input.now_ns.max(host.now_ns());
    let margin_ns = params.deadline_margin_s.saturating_mul(NS_PER_S);
    let horizon = now.saturating_add(margin_ns);

    // 1. Load the book.
    let stored = host.storage_get(BOOK_KEY).map_err(|_| "storage_unavailable")?;
    let mut book = match &stored {
        Some(bytes) => Book::decode(bytes)?,
        None => Book::default(),
    };

    // 2. Open the new orders (pure checks only). How many fit is decided after
    //    reconciliation frees room; here the candidates are only bounded so
    //    their live checks stay a bounded wave.
    let key = MatcherKey::from_seed(host.x25519_seed().map_err(|_| "key_unavailable")?);
    let mut fates: Vec<(String, Fate)> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    let (mut n_orders, mut n_lots, mut n_bytes) = (0usize, 0usize, 0usize);
    let mut known_nonces = book.nonces();
    let mut seen = BTreeSet::new();
    for o in &input.orders {
        if !seen.insert(o.id.clone()) {
            continue;
        }
        if book.contains_order(&o.id) {
            fates.push((o.id.clone(), Fate::Ingested));
            continue;
        }
        let fate = match ingest::open(&key, o, input.market(&o.market), verifier, horizon) {
            Err(r) => Fate::Rejected(r),
            Ok(c) => {
                let owner = &c.order.meta.owner;
                if c.order
                    .lots
                    .iter()
                    .any(|l| known_nonces.contains(&(owner.clone(), l.nonce.clone())))
                {
                    Fate::Rejected(Reject::Invalid)
                } else if n_orders + 1 > MAX_ORDERS
                    || n_lots + c.order.lots.len() > MAX_LOTS
                    || n_bytes + c.bytes > MAX_BOOK_BYTES / 2
                {
                    Fate::Deferred
                } else {
                    n_orders += 1;
                    n_lots += c.order.lots.len();
                    n_bytes += c.bytes;
                    known_nonces.extend(c.order.lots.iter().map(|l| (owner.clone(), l.nonce.clone())));
                    candidates.push(c);
                    Fate::Pending(candidates.len() - 1)
                }
            }
        };
        fates.push((o.id.clone(), fate));
    }

    // 3. Wave 1: every view and quote the batch depends on.
    let mut w = Wave::default();
    let view =
        |w: &mut Wave, method: &'static str, args: Value| w.push(View::new(verifier, method, args).request(rpc_url));
    let i_salt = view(&mut w, "current_salt", json!({}));
    let i_fee = view(&mut w, "fee", json!({}));

    let mut salts: BTreeSet<String> = BTreeSet::new();
    let mut nonces: BTreeSet<(String, String)> = BTreeSet::new();
    let mut balances: BTreeSet<(String, String)> = BTreeSet::new();
    for o in &book.orders {
        for l in &o.lots {
            salts.extend(lot_salt(l));
            nonces.insert((o.meta.owner.clone(), l.nonce.clone()));
        }
        let token = if o.meta.side == Side::Sell {
            &o.meta.base
        } else {
            &o.meta.quote
        };
        balances.insert((o.meta.owner.clone(), token.clone()));
    }
    for f in &book.in_flight {
        salts.extend(lot_salt(&f.lot));
        nonces.insert((f.meta.owner.clone(), f.lot.nonce.clone()));
        let token = if f.meta.side == Side::Sell {
            &f.meta.base
        } else {
            &f.meta.quote
        };
        balances.insert((f.meta.owner.clone(), token.clone()));
    }
    let mut keys: BTreeSet<(String, String)> = BTreeSet::new();
    for c in &candidates {
        let m = &c.order.meta;
        salts.extend(c.salts.iter().cloned());
        nonces.extend(c.order.lots.iter().map(|l| (m.owner.clone(), l.nonce.clone())));
        keys.extend(c.keys.iter().map(|k| (m.owner.clone(), k.clone())));
        balances.insert((
            m.owner.clone(),
            if m.side == Side::Sell {
                m.base.clone()
            } else {
                m.quote.clone()
            },
        ));
    }
    for m in &input.markets {
        balances.insert((input.venue_account.clone(), m.base.clone()));
        balances.insert((input.venue_account.clone(), m.quote.clone()));
    }
    let salt_idx: BTreeMap<String, usize> = salts
        .into_iter()
        .map(|s| {
            let i = view(&mut w, "is_valid_salt", json!({ "salt": s }));
            (s, i)
        })
        .collect();
    let nonce_idx: BTreeMap<(String, String), usize> = nonces
        .into_iter()
        .map(|(a, n)| {
            let i = view(&mut w, "is_nonce_used", json!({ "account_id": a, "nonce": n }));
            ((a, n), i)
        })
        .collect();
    let key_idx: BTreeMap<(String, String), usize> = keys
        .into_iter()
        .map(|(a, k)| {
            let i = view(&mut w, "has_public_key", json!({ "account_id": a, "public_key": k }));
            ((a, k), i)
        })
        .collect();
    let balance_idx: BTreeMap<(String, String), usize> = balances
        .into_iter()
        .map(|(a, t)| {
            let i = view(&mut w, "mt_balance_of", json!({ "account_id": a, "token_id": t }));
            ((a, t), i)
        })
        .collect();
    let quote_idx: Vec<(usize, usize)> = input
        .markets
        .iter()
        .map(|m| {
            let r = &m.reference;
            (
                w.push(oneclick::request(&r.base, &r.quote, r.probe_base, now)),
                w.push(oneclick::request(&r.quote, &r.base, r.probe_quote, now)),
            )
        })
        .collect();
    let resp = w.send(host)?;

    let current_salt: Option<[u8; 4]> = opt(rpc::parse(&resp[i_salt])).and_then(|v| v.as_str().and_then(salt_bytes));
    let fee_pips: Option<u32> = opt(rpc::parse(&resp[i_fee]))
        .and_then(|v| v.as_u64())
        .filter(|f| *f < 1_000_000)
        .map(|f| f as u32);
    let salt_ok = |s: &str| salt_idx.get(s).and_then(|i| opt(rpc::parse_bool(&resp[*i])));
    let nonce_used = |a: &str, n: &str| {
        nonce_idx
            .get(&(a.to_string(), n.to_string()))
            .and_then(|i| opt(rpc::parse_bool(&resp[*i])))
    };
    let balance_of = |a: &str, t: &str| {
        balance_idx
            .get(&(a.to_string(), t.to_string()))
            .and_then(|i| opt(rpc::parse_u128(&resp[*i])))
    };

    // 4. Reconcile in-flight lots: a used nonce is a fill; an unused one goes
    //    back to the book once its sub-batch can no longer settle.
    for f in std::mem::take(&mut book.in_flight) {
        match nonce_used(&f.meta.owner, &f.lot.nonce) {
            Some(true) => {}
            Some(false) if now > f.settle_by_ns.saturating_add(SETTLE_GUARD_NS) => {
                if f.lot.deadline_ns > now {
                    book.restore(f);
                }
            }
            _ => book.in_flight.push(f),
        }
    }

    // 5. Reconcile resting lots: a used nonce is a cancel ticket, an
    //    invalidated salt or a passed deadline can never settle. A lot crosses
    //    this round only if both facts are known good.
    let mut crossable: BTreeSet<(String, u16)> = BTreeSet::new();
    for o in book.orders.iter_mut() {
        let (id, owner) = (o.meta.id.clone(), o.meta.owner.clone());
        o.lots.retain(|l| {
            let used = nonce_used(&owner, &l.nonce);
            let salt = lot_salt(l).and_then(|s| salt_ok(&s));
            if used == Some(true) || salt == Some(false) || l.deadline_ns <= now {
                return false;
            }
            if used == Some(false) && salt == Some(true) {
                crossable.insert((id.clone(), l.index));
            }
            true
        });
    }
    book.prune();

    // 6. Ingest: the live checks decide between ingested, rejected and
    //    deferred; a valid order that does not fit the book waits at the pool.
    let mut ingested = Vec::new();
    let mut rejected = Vec::new();
    let mut deferred = Vec::new();
    let mut book_bytes = book.encode()?.len();
    for (id, fate) in fates {
        let fate = match fate {
            Fate::Pending(ci) => {
                let c = &candidates[ci];
                let owner = c.order.meta.owner.as_str();
                let lots: Vec<Option<bool>> = c.order.lots.iter().map(|l| nonce_used(owner, &l.nonce)).collect();
                let salts: Vec<Option<bool>> = c.salts.iter().map(|s| salt_ok(s)).collect();
                let keys: Vec<Option<bool>> = c
                    .keys
                    .iter()
                    .map(|k| {
                        key_idx
                            .get(&(owner.to_string(), k.clone()))
                            .and_then(|i| opt(rpc::parse_bool(&resp[*i])))
                    })
                    .collect();
                let n = book.counts();
                let fits = n.orders < MAX_ORDERS
                    && n.lots + c.order.lots.len() <= MAX_LOTS
                    && book_bytes + c.bytes <= MAX_BOOK_BYTES / 2;
                let unknown = lots.contains(&None) || salts.contains(&None) || keys.contains(&None);
                if lots.contains(&Some(true)) || salts.contains(&Some(false)) || keys.contains(&Some(false)) {
                    Fate::Rejected(Reject::Invalid)
                } else if unknown || !fits {
                    Fate::Deferred
                } else {
                    book_bytes += c.bytes;
                    crossable.extend(c.order.lots.iter().map(|l| (c.order.meta.id.clone(), l.index)));
                    book.orders.push(c.order.clone());
                    Fate::Ingested
                }
            }
            other => other,
        };
        match fate {
            Fate::Ingested => ingested.push(id),
            Fate::Rejected(r) => rejected.push(Rejected { id, reason: r.code() }),
            Fate::Deferred | Fate::Pending(_) => deferred.push(id),
        }
    }
    book.prune();

    // 7. Exclude orders their owner cannot pay for, in time priority, counting
    //    every eligible order of the owner against the same balance.
    let mut eligible: BTreeSet<String> = BTreeSet::new();
    let mut committed: BTreeMap<(String, String), u128> = BTreeMap::new();
    for o in &book.orders {
        let m = &o.meta;
        let lots: Vec<_> = o
            .lots
            .iter()
            .filter(|l| crossable.contains(&(m.id.clone(), l.index)))
            .collect();
        if lots.is_empty() || lots.len() < m.min_lots as usize {
            continue;
        }
        let (token, need) = match m.side {
            Side::Sell => (&m.base, (lots.len() as u128).checked_mul(m.lot)),
            Side::Buy => (
                &m.quote,
                lots.iter().try_fold(0u128, |a, l| a.checked_add(l.limit_quote)),
            ),
        };
        let (Some(need), Some(balance)) = (need, balance_of(&m.owner, token)) else {
            continue;
        };
        let used = committed.entry((m.owner.clone(), token.clone())).or_default();
        if used.checked_add(need).is_some_and(|total| total <= balance) {
            *used += need;
            eligible.insert(m.id.clone());
        }
    }

    // 8. Price, cross, plan and sign, market by market.
    let deadline_s = now / NS_PER_S + VENUE_TTL_S;
    let settle_by_ns = deadline_s * NS_PER_S;
    let mut entries: Vec<MarketBatch> = input.markets.iter().map(|m| MarketBatch::new(&m.id)).collect();
    let mut planned: Vec<Planned> = Vec::new();
    let mut subs_left = params.max_sub_batches as usize;
    let mut venue_spend: BTreeMap<String, u128> = BTreeMap::new();
    for (mi, m) in input.markets.iter().enumerate() {
        let entry = &mut entries[mi];
        let r = &m.reference;
        let max_age = input.quote_max_age_s();
        let bid = oneclick::verify(&resp[quote_idx[mi].0], &r.base, &r.quote, r.probe_base, now, max_age);
        let ask = oneclick::verify(&resp[quote_idx[mi].1], &r.quote, &r.base, r.probe_quote, now, max_age);
        let (bid, ask) = match (bid, ask) {
            (Ok(b), Ok(a)) => (b, a),
            (Err(e), _) | (_, Err(e)) => {
                entry.skipped = Some(quote_skip(e));
                continue;
            }
        };
        entry.quotes = vec![bid.digest_b58.clone(), ask.digest_b58.clone()];
        let (bid_px, ask_px) = match oneclick::bid_ask(&bid, &ask, &r.base, &r.quote) {
            Ok(p) => p,
            Err(e) => {
                entry.skipped = Some(quote_skip(e));
                continue;
            }
        };
        let (Some(fee_pips), Some(salt)) = (fee_pips, current_salt) else {
            entry.skipped = Some("rpc_unavailable");
            continue;
        };

        let market = Market {
            base: m.base.clone(),
            quote: m.quote.clone(),
            lot: m.lot,
        };
        let orders: Vec<Order> = book
            .orders
            .iter()
            .filter(|o| {
                let x = &o.meta;
                x.market == m.id && x.base == m.base && x.quote == m.quote && x.lot == m.lot && eligible.contains(&x.id)
            })
            .map(|o| {
                let lots: Vec<Lot> = o
                    .lots
                    .iter()
                    .filter(|l| crossable.contains(&(o.meta.id.clone(), l.index)))
                    .map(|l| Lot {
                        index: l.index,
                        limit_quote: l.limit_quote,
                        intent: l.intent.clone(),
                    })
                    .collect();
                let deadline_ns = o
                    .lots
                    .iter()
                    .filter(|l| crossable.contains(&(o.meta.id.clone(), l.index)))
                    .map(|l| l.deadline_ns)
                    .min()
                    .unwrap_or(0);
                Order {
                    id: o.meta.id.clone(),
                    owner: o.meta.owner.clone(),
                    side: o.meta.side,
                    lots,
                    min_lots: o.meta.min_lots,
                    deadline_ns,
                    seq: o.meta.seq,
                }
            })
            .collect();
        let engine_params = blindbook_core::Params {
            fee_bps: params.fee_bps,
            max_spread_bps: params.max_spread_bps,
            deadline_margin_ns: margin_ns,
        };
        let batch = match cross(&market, &orders, bid_px, ask_px, now, &engine_params) {
            Ok(b) => b,
            Err(_) => {
                entry.skipped = Some("bad_reference");
                continue;
            }
        };
        entry.mid = batch.mid;
        entry.lot_mid_quote = batch.lot_mid_quote.to_string();
        match &batch.skipped {
            Some(SkipReason::BadReference { .. }) => {
                entry.skipped = Some("bad_reference");
                continue;
            }
            Some(SkipReason::NoCross) => {
                entry.skipped = Some("no_cross");
                continue;
            }
            None => {}
        }

        let step = params.max_lots_per_side as usize;
        let subs = plan(&batch, &market, fee_pips, step);
        let chunks = batch.sells.chunks(step).zip(batch.buys.chunks(step));
        for (sub, (sells, buys)) in subs.into_iter().zip(chunks) {
            if subs_left == 0 {
                break;
            }
            // The venue pays base dust (and any quote shortfall) out of its own
            // inventory; every included sub-batch draws on the same balance.
            let needs = [(&m.base, sub.venue_base_net), (&m.quote, sub.venue_quote_net)];
            let affordable = needs.iter().all(|(token, net)| {
                if *net >= 0 {
                    return true;
                }
                let spent = venue_spend.get(*token).copied().unwrap_or(0);
                match balance_of(&input.venue_account, token) {
                    Some(b) => spent.saturating_add(net.unsigned_abs()) <= b,
                    None => false,
                }
            });
            if !affordable {
                entry.dropped.push("venue_inventory");
                continue;
            }
            for (token, net) in needs {
                if net < 0 {
                    *venue_spend.entry(token.clone()).or_default() += net.unsigned_abs();
                }
            }
            let venue = venue_payload(
                host,
                &input.venue_account,
                verifier,
                salt,
                deadline_s,
                sub.venue_intents(&market),
            )?;
            let mut signed = sub.user_intents.clone();
            signed.push(venue);
            planned.push(Planned {
                market: mi,
                sub,
                sells: sells.to_vec(),
                buys: buys.to_vec(),
                lot_mid: batch.lot_mid_quote,
                lot: m.lot,
                signed,
            });
            subs_left -= 1;
        }
    }

    // 9. Wave 2: simulate every planned sub-batch against the live verifier.
    let mut w = Wave::default();
    for p in &planned {
        w.push(View::new(verifier, "simulate_intents", json!({ "signed": p.signed })).request(rpc_url));
    }
    let sims: Vec<Sim> = w
        .send(host)?
        .iter()
        .zip(&planned)
        .map(|(r, p)| classify_simulation(r, p.signed.len()))
        .collect();

    // 10. Wave 3: for a sub-batch the verifier refused, simulate each user lot
    //     alone. A lot that cannot even run alone (bad salt, removed key, spent
    //     nonce, missing funds) earns its order a strike.
    let mut w = Wave::default();
    let mut solo: Vec<String> = Vec::new();
    for (p, sim) in planned.iter().zip(&sims) {
        if *sim == Sim::Panicked {
            for l in p.sells.iter().chain(&p.buys) {
                w.push(View::new(verifier, "simulate_intents", json!({ "signed": [l.intent] })).request(rpc_url));
                solo.push(l.order_id.clone());
            }
        }
    }
    let struck: BTreeSet<String> = w
        .send(host)?
        .iter()
        .zip(solo)
        .filter(|(r, _)| matches!(rpc::parse(r), Err(ViewError::Contract(_))))
        .map(|(_, id)| id)
        .collect();

    // 11. Apply: settle what passed, mark its lots in flight.
    let mut settle = Vec::new();
    let mut stats: Vec<[u128; 5]> = vec![[0; 5]; entries.len()];
    for (p, sim) in planned.into_iter().zip(sims) {
        let entry = &mut entries[p.market];
        match sim {
            Sim::Pass => {}
            Sim::Unavailable => {
                entry.dropped.push("simulate_unavailable");
                continue;
            }
            Sim::Failed | Sim::Panicked => {
                entry.dropped.push("simulate_failed");
                continue;
            }
        }
        let n = p.sub.lots_per_side as u128;
        let s = &mut stats[p.market];
        s[0] += n;
        s[1] = s[1].saturating_add(n.saturating_mul(p.lot));
        s[2] = s[2].saturating_add(n.saturating_mul(p.lot_mid));
        s[3] = s[3].saturating_add(p.sub.rebates.iter().map(|r| r.amount).fold(0u128, u128::saturating_add));
        s[4] = s[4].saturating_add(
            p.sells
                .iter()
                .chain(&p.buys)
                .map(|l| l.fee)
                .fold(0u128, u128::saturating_add),
        );
        entry.sub_batches += 1;
        for l in p.sells.iter().chain(&p.buys) {
            let Some(o) = book.orders.iter_mut().find(|o| o.meta.id == l.order_id) else {
                return Err("internal");
            };
            let Some(pos) = o.lots.iter().position(|x| x.index == l.index) else {
                return Err("internal");
            };
            let lot = o.lots.remove(pos);
            book.in_flight.push(InFlight {
                meta: o.meta.clone(),
                lot,
                settle_by_ns,
            });
        }
        settle.push(p.signed);
    }
    for (entry, s) in entries.iter_mut().zip(stats) {
        entry.matched_lots = s[0] as u32;
        entry.base_volume = s[1].to_string();
        entry.quote_volume = s[2].to_string();
        entry.rebated = s[3].to_string();
        entry.venue_fee = s[4].to_string();
    }
    for o in book.orders.iter_mut() {
        if struck.contains(&o.meta.id) {
            o.strikes = o.strikes.saturating_add(1);
        }
    }
    book.orders.retain(|o| o.strikes < MAX_STRIKES);
    book.prune();

    let counts = book.counts();
    let book_bytes = book.encode()?;
    Ok(Outcome {
        result: BatchResult {
            v: 1,
            op: "batch",
            error: None,
            input_sha256,
            now_ns: now.to_string(),
            ingested,
            rejected,
            deferred,
            batches: entries,
            settle,
            book: Some(BookCounts {
                orders: counts.orders,
                lots: counts.lots,
            }),
        },
        book_bytes,
        stored,
    })
}
