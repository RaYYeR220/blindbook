//! # Blindbook pool
//!
//! The on-chain half of Blindbook (docs/PROTOCOL.md §2 and §3). Traders store
//! sealed orders here; anyone cranks a batch, which sends the pending
//! ciphertexts to the matcher, an OutLayer project running in a TEE. The
//! matcher's signed result comes back through OutLayer's yield/resume: the pool
//! checks the signature against a pinned key, refunds the storage deposits of
//! the orders the matcher ingested or rejected, records the batch, and sends
//! each settlement sub-batch to the NEAR Intents verifier's `execute_intents`.
//!
//! ## What the pool never does
//! It never holds trader funds: the only NEAR it keeps for traders is the
//! storage deposit of each pending order, always refunded to the submitter.
//! There is no pause, no withdraw and no admin path to user funds. Settlement
//! moves tokens only inside the verifier, and only as the traders' own signed
//! lots allow (each lot carries its limit price; the verifier applies it exactly).
//!
//! ## Trust points (owner-only, every change emits `config_changed`)
//! - `set_project(project_id, version_key)`: which OutLayer project and pinned
//!   wasm hash runs the matcher. A different build could see order plaintexts.
//!   Switching to another project clears the pinned result, matcher and venue
//!   keys: OutLayer host keys are bound to the project, so they must be re-pinned.
//! - `set_result_key(key)`: the ed25519 key whose signature the pool accepts on
//!   results. It must be the matcher's TEE-held key (check the attestation).
//!
//! ## Key bootstrap (owner pins; no trust on first use)
//! The matcher's keys exist only inside runs made through this pool (they are
//! bound to the calling contract), so a `run_keys` is how they are discovered.
//! While no result key is pinned, a keys result is recorded in the batch record
//! (`keys`, status `unpinned`) and nothing in it is trusted. The owner checks it
//! against the run's TDX attestation and calls `set_result_key`; the next
//! `run_keys` is then verified and publishes `matcher_key` and (if unset) the
//! venue key. Batch results are refused while no key is pinned.
//! - `set_venue(account, key)`: the verifier account the matcher settles through.
//! - `add_market` / `remove_market` / `set_params` / `set_run_deposit`: market
//!   list, matcher parameters, and the minimum OutLayer deposit for a crank.
//!
//! The account's own access keys (contract redeploys) are outside this
//! contract and are a trust point too.
//!
//! ## Cranking
//! `run_batch` and `run_keys` are open to anyone. Attach at least
//! `get_config().run_deposit` (default 0.005 NEAR; OutLayer refunds the unused
//! part to the caller) and at least `get_config().run_gas` of prepaid gas
//! (435 TGas with `max_sub_batches = 4`; formula in [`run_gas`]). A deposit
//! above `run_deposit` raises the input budget past OutLayer's free 32 KiB at
//! 0.00001 NEAR per byte, held by OutLayer and refunded.

use near_sdk::base64::engine::general_purpose::STANDARD as B64;
use near_sdk::base64::Engine;
use near_sdk::json_types::{U128, U64};
use near_sdk::serde::Serialize;
use near_sdk::store::LookupMap;
use near_sdk::{
    env, near, require, AccountId, Gas, GasWeight, NearToken, PanicOnDefault, Promise,
    PromiseError, PromiseOrValue,
};

pub mod events;
pub mod matcher;
pub mod types;

#[cfg(feature = "migrate-v0")]
mod migrate;
#[cfg(test)]
mod tests;

use events::emit;
use matcher::{
    clip, execute_intents_args, hex, input_order_len, is_x25519_hex, normalize_ed25519_key,
    parse_ed25519_key, parse_ed25519_sig, BatchResult, Input, InputOrder, KeysResult, Output,
    OP_BATCH, OP_KEYS,
};
pub use types::*;

// ---------------------------------------------------------------- limits

/// Envelope = version (1) + ephemeral key (32) + nonce (24) + AEAD tag (16) + padded plaintext.
pub const ENVELOPE_OVERHEAD: usize = 1 + 32 + 24 + 16;
/// Padded plaintext sizes (PROTOCOL §1.3).
pub const BUCKETS: [usize; 6] = [2_048, 4_096, 8_192, 16_384, 32_768, 65_536];
pub const MAX_PENDING: usize = 64;
pub const MAX_ORDERS_PER_BATCH: usize = 32;
/// The matcher prices at most 8 markets per input (PROTOCOL §3.4).
pub const MAX_MARKETS: usize = 8;
pub const MAX_MARKET_ID_LEN: usize = 32;
pub const MAX_TOKEN_LEN: usize = 128;
pub const MAX_REASON_LEN: usize = 64;

/// 0.00001 NEAR, the chain's storage price per byte.
pub const STORAGE_PRICE_PER_BYTE: u128 = 10_000_000_000_000_000_000;
/// Bytes charged on top of the measured storage delta of an order: its index
/// entry in contract state plus slack.
pub const ORDER_STORAGE_BUFFER: u64 = 64;
/// Upper bound of an order's storage beyond its envelope, used by `get_order_deposit`.
pub const ORDER_DEPOSIT_OVERHEAD: u64 = 256;

/// Input OutLayer stores for free; above it every byte is held at the storage price.
pub const OUTLAYER_FREE_INPUT: usize = 32 * 1024;
/// Input budget when the crank attaches no more than `run_deposit`.
pub const SOFT_INPUT_BYTES: usize = 30_000;
/// Hard input cap, whatever the deposit.
pub const MAX_INPUT_BYTES: usize = 160_000;
/// Largest callback payload read; anything longer is treated as a failed run.
pub const MAX_RESULT_BYTES: usize = 512 * 1024;

/// OutLayer's yield window is 200 blocks; past this a stuck run no longer blocks cranks.
pub const GUARD_TIMEOUT_BLOCKS: u64 = 250;
/// Batch records kept on chain (events carry the full history).
pub const BATCH_RETENTION: u64 = 128;
pub const DEFAULT_RUN_DEPOSIT: u128 = 5_000_000_000_000_000_000_000; // 0.005 NEAR
pub const MAX_SUB_BATCHES_CAP: u32 = 12;

pub const MAX_INSTRUCTIONS: u64 = 20_000_000_000;
pub const MAX_MEMORY_MB: u32 = 256;
pub const MAX_EXECUTION_SECONDS: u64 = 30;

// ---------------------------------------------------------------- gas

/// Static gas for `outlayer.request_execution` (also gets the unused-gas share).
pub const REQUEST_GAS: Gas = Gas::from_tgas(100);
/// Static gas per `execute_intents` sub-batch (13 intents measured at ~41 TGas).
pub const SETTLE_GAS: Gas = Gas::from_tgas(50);
pub const ON_SETTLED_GAS: Gas = Gas::from_tgas(15);
/// `on_result`'s own work: parse, verify, refund transfers, records.
pub const ON_RESULT_BASE_GAS: Gas = Gas::from_tgas(60);
/// Fees burnt by the caller for each function-call receipt it creates.
pub const RECEIPT_OVERHEAD_GAS: Gas = Gas::from_tgas(5);
/// `run_batch`'s own work: read orders, build input, write records.
pub const RUN_OWN_GAS: Gas = Gas::from_tgas(30);

/// Static gas attached to `on_result` so it can fan out `k` sub-batches.
pub const fn on_result_gas(k: u32) -> Gas {
    let per = SETTLE_GAS.as_gas() + RECEIPT_OVERHEAD_GAS.as_gas();
    Gas::from_gas(
        ON_RESULT_BASE_GAS.as_gas()
            + per * k as u64
            + ON_SETTLED_GAS.as_gas()
            + RECEIPT_OVERHEAD_GAS.as_gas(),
    )
}

/// Prepaid gas `run_batch` / `run_keys` require: 135 TGas + `on_result_gas(k)`,
/// i.e. 435 TGas for k = 4 and 875 TGas for k = 12 (the 1 PGas tx limit allows it).
pub const fn run_gas(k: u32) -> Gas {
    Gas::from_gas(
        RUN_OWN_GAS.as_gas()
            + REQUEST_GAS.as_gas()
            + RECEIPT_OVERHEAD_GAS.as_gas()
            + on_result_gas(k).as_gas(),
    )
}

/// Deposit OutLayer holds for `len` bytes of input beyond its free allowance.
pub fn input_storage_deposit(len: usize) -> u128 {
    len.saturating_sub(OUTLAYER_FREE_INPUT) as u128 * STORAGE_PRICE_PER_BYTE
}

pub fn valid_envelope_len(len: usize) -> bool {
    BUCKETS.iter().any(|b| len == ENVELOPE_OVERHEAD + b)
}

// ---------------------------------------------------------------- state

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Pool {
    owner: AccountId,
    network: String,
    outlayer: AccountId,
    project_id: String,
    version_key: String,
    result_key: Option<String>,
    matcher_key: Option<String>,
    venue_account: AccountId,
    venue_key: Option<String>,
    verifier: AccountId,
    markets: Vec<Market>,
    params: Params,
    run_deposit: u128,
    /// Pending order ids in submission order, with envelope lengths.
    pending: Vec<PendingEntry>,
    orders: LookupMap<u64, Order>,
    next_seq: u64,
    next_batch_id: u64,
    batches: LookupMap<u64, BatchRecord>,
    in_flight: Option<InFlight>,
    stats: Vec<MarketStats>,
    totals: Totals,
}

// ---------------------------------------------------------------- event payloads

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct OrderSubmitted<'a> {
    id: U64,
    seq: u64,
    market: &'a str,
    submitter: &'a AccountId,
    bytes: u32,
    deposit: U128,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct OrderCancelled<'a> {
    id: U64,
    submitter: &'a AccountId,
    refund: U128,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct BatchRequested<'a> {
    batch_id: U64,
    op: &'a str,
    cranker: &'a AccountId,
    orders: &'a [U64],
    input_bytes: u32,
    deposit: U128,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct MarketLine<'a> {
    market: &'a str,
    matched_lots: u32,
    base_volume: U128,
    quote_volume: U128,
    venue_fee: U128,
    mid: &'a Option<Mid>,
    skipped: &'a Option<String>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct BatchResultEvent<'a> {
    batch_id: U64,
    op: &'a str,
    status: BatchStatus,
    error: &'a Option<String>,
    result_hash: &'a Option<String>,
    ingested: &'a [U64],
    rejected: &'a [Rejection],
    deferred: &'a [U64],
    refunded: U128,
    markets: Vec<MarketLine<'a>>,
    book: &'a Option<BookCounts>,
    sub_batches: u32,
    dispatched: u32,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct BatchSettled<'a> {
    batch_id: U64,
    status: BatchStatus,
    sub_batches: u32,
    settled: u32,
    failed: u32,
    results: &'a [bool],
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct KeysPublished<'a> {
    batch_id: U64,
    matcher_x25519: &'a str,
    venue: &'a str,
    result_key: &'a str,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct ConfigChanged<'a, T: Serialize> {
    field: &'a str,
    by: &'a AccountId,
    value: T,
}

type Terminal = (BatchStatus, String);

// ---------------------------------------------------------------- contract

#[near]
impl Pool {
    #[init]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: AccountId,
        network: String,
        outlayer: AccountId,
        project_id: String,
        version_key: String,
        result_key: Option<String>,
        venue_account: AccountId,
        venue_key: Option<String>,
        verifier: AccountId,
        markets: Vec<Market>,
        params: Params,
    ) -> Self {
        require!(
            network == "mainnet" || network == "testnet",
            "network must be mainnet or testnet"
        );
        require!(markets.len() <= MAX_MARKETS, "too many markets");
        assert_project(&project_id, &version_key);
        assert_params(&params);
        let mut pool = Self {
            owner,
            network,
            outlayer,
            project_id,
            version_key,
            result_key: result_key.map(|k| key_or_panic(&k)),
            matcher_key: None,
            venue_account,
            venue_key: venue_key.map(|k| key_or_panic(&k)),
            verifier,
            markets: Vec::new(),
            params,
            run_deposit: DEFAULT_RUN_DEPOSIT,
            pending: Vec::new(),
            orders: LookupMap::new(b"o"),
            next_seq: 0,
            next_batch_id: 0,
            batches: LookupMap::new(b"b"),
            in_flight: None,
            stats: Vec::new(),
            totals: Totals::default(),
        };
        for m in markets {
            assert_market(&m);
            require!(pool.market(&m.id).is_none(), "duplicate market id");
            pool.markets.push(m);
        }
        pool
    }

    // ------------------------------------------------------------ orders

    /// Stores a sealed order. Attach at least its storage cost
    /// (`get_order_deposit(bytes)` is a safe upper bound); the excess comes
    /// back at once, the rest when the matcher ingests or rejects the order.
    #[payable]
    pub fn submit_order(&mut self, market: String, sealed_b64: String) -> U64 {
        require!(self.market(&market).is_some(), "unknown market");
        let sealed = B64
            .decode(sealed_b64.as_bytes())
            .unwrap_or_else(|_| env::panic_str("sealed_b64 is not valid base64"));
        require!(
            valid_envelope_len(sealed.len()),
            "sealed order must be 73 bytes + a 2, 4, 8, 16, 32 or 64 KiB bucket"
        );
        require!(
            self.pending.len() < MAX_PENDING,
            "pending queue is full; wait for the next batch"
        );

        let submitter = env::predecessor_account_id();
        let attached = env::attached_deposit().as_yoctonear();
        let id = self.next_seq;
        self.next_seq += 1;
        let bytes = sealed.len() as u32;

        // Measure the record's real footprint; the deposit field's size is fixed.
        let before = env::storage_usage();
        self.orders.insert(
            id,
            Order {
                market: market.clone(),
                submitter: submitter.clone(),
                sealed,
                deposit: 0,
                submitted_at: env::block_timestamp(),
            },
        );
        self.orders.flush();
        let used = env::storage_usage().saturating_sub(before) + ORDER_STORAGE_BUFFER;
        let required = used as u128 * env::storage_byte_cost().as_yoctonear();
        require!(
            attached >= required,
            format!("attach at least {required} yoctoNEAR to cover the order's storage")
        );
        if let Some(o) = self.orders.get_mut(&id) {
            o.deposit = required;
        }
        self.pending.push(PendingEntry { id, len: bytes });
        if attached > required {
            Promise::new(submitter.clone())
                .transfer(NearToken::from_yoctonear(attached - required))
                .detach();
        }
        emit(
            "order_submitted",
            &OrderSubmitted {
                id: U64(id),
                seq: id,
                market: &market,
                submitter: &submitter,
                bytes,
                deposit: U128(required),
            },
        );
        U64(id)
    }

    /// Removes a pending order the matcher has not been sent yet and refunds
    /// its deposit. After ingestion, cancel lots with cancel tickets (§1.4).
    pub fn cancel_order(&mut self, id: U64) -> U128 {
        let id = id.0;
        require!(
            self.pending.iter().any(|e| e.id == id),
            "order is not pending"
        );
        let submitter = self
            .orders
            .get(&id)
            .map(|o| o.submitter.clone())
            .unwrap_or_else(|| env::abort());
        require!(
            env::predecessor_account_id() == submitter,
            "only the submitter can cancel"
        );
        require!(
            !self.in_flight_live_contains(id),
            "order is in the running batch"
        );
        let order = self.take_pending(id).unwrap_or_else(|| env::abort());
        if order.deposit > 0 {
            Promise::new(submitter.clone())
                .transfer(NearToken::from_yoctonear(order.deposit))
                .detach();
        }
        emit(
            "order_cancelled",
            &OrderCancelled {
                id: U64(id),
                submitter: &submitter,
                refund: U128(order.deposit),
            },
        );
        U128(order.deposit)
    }

    // ------------------------------------------------------------ crank

    /// Sends pending orders to the matcher. Anyone may call; see the crate
    /// docs for the deposit and gas it needs. Orders are taken in submission
    /// order while they fit the input budget; larger ones wait for a crank
    /// that attaches more deposit.
    #[payable]
    pub fn run_batch(&mut self) -> Promise {
        let budget = self.input_budget();
        let (ids, input) = self.build_input(OP_BATCH, budget);
        self.dispatch(OP_BATCH, ids, input)
    }

    /// Asks the matcher to publish its keys (`op: "keys"`). Anyone may call.
    #[payable]
    pub fn run_keys(&mut self) -> Promise {
        let budget = self.input_budget();
        let (ids, input) = self.build_input(OP_KEYS, budget);
        self.dispatch(OP_KEYS, ids, input)
    }

    /// OutLayer's answer: the module's stdout JSON, or `null` on failure.
    #[private]
    pub fn on_result(
        &mut self,
        #[serializer(borsh)] batch_id: u64,
        #[serializer(borsh)] order_ids: Vec<u64>,
    ) -> PromiseOrValue<BatchStatus> {
        let mut rec = self
            .batches
            .get(&batch_id)
            .cloned()
            .unwrap_or_else(|| env::abort());
        if self
            .in_flight
            .as_ref()
            .is_some_and(|f| f.batch_id == batch_id)
        {
            self.in_flight = None;
        }
        rec.finished_at = Some(U64(env::block_timestamp()));
        rec.error = None;

        let settle = match self.verify_and_apply(&mut rec, &order_ids) {
            Ok(settle) => settle,
            Err((status, error)) => {
                rec.status = status;
                rec.error = Some(error);
                None
            }
        };
        if matches!(
            rec.status,
            BatchStatus::Failed | BatchStatus::RejectedSignature | BatchStatus::InvalidResult
        ) {
            self.totals.failed += 1;
        }
        self.emit_batch_result(&rec);
        let status = rec.status;
        self.batches.insert(batch_id, rec);
        match settle {
            Some(p) => PromiseOrValue::Promise(p),
            None => PromiseOrValue::Value(status),
        }
    }

    /// Outcome of the `execute_intents` calls of one batch.
    #[private]
    pub fn on_settled(
        &mut self,
        #[serializer(borsh)] batch_id: u64,
        #[serializer(borsh)] n: u32,
    ) -> BatchStatus {
        let mut rec = self
            .batches
            .get(&batch_id)
            .cloned()
            .unwrap_or_else(|| env::abort());
        let count = env::promise_results_count().min(n as u64);
        let results: Vec<bool> = (0..count)
            .map(|i| !matches!(env::promise_result_checked(i, 0), Err(PromiseError::Failed)))
            .collect();
        let ok = results.iter().filter(|r| **r).count() as u32;
        let failed = results.len() as u32 - ok;
        rec.status = if ok == rec.sub_batches && failed == 0 {
            BatchStatus::Settled
        } else if ok == 0 {
            BatchStatus::SettlementFailed
        } else {
            BatchStatus::PartiallySettled
        };
        self.totals.sub_batches_settled += ok as u64;
        self.totals.sub_batches_failed += failed as u64;
        // A market's volumes count once all of its sub-batches settled. `settle`
        // is grouped by market in `batches` order with `sub_batches` each
        // (§3.4); if those counts do not add up, count only a fully settled batch.
        let spans: u32 = rec.markets.iter().map(|m| m.sub_batches).sum();
        let mut counted: Vec<usize> = Vec::new();
        if spans == rec.sub_batches {
            let mut at = 0usize;
            for (i, m) in rec.markets.iter().enumerate() {
                let span = m.sub_batches as usize;
                if span > 0
                    && results
                        .get(at..at + span)
                        .is_some_and(|r| r.iter().all(|ok| *ok))
                {
                    counted.push(i);
                }
                at += span;
            }
        } else if rec.status == BatchStatus::Settled {
            counted = (0..rec.markets.len()).collect();
        }
        for i in counted {
            let m = &rec.markets[i];
            if m.matched_lots == 0 {
                continue;
            }
            let s = self.stats_mut(&m.market);
            s.batches += 1;
            s.crossed_lots += m.matched_lots as u64;
            s.base_volume = U128(s.base_volume.0.saturating_add(m.base_volume.0));
            s.quote_volume = U128(s.quote_volume.0.saturating_add(m.quote_volume.0));
            s.venue_fees = U128(s.venue_fees.0.saturating_add(m.venue_fee.0));
        }
        rec.settled = results;
        emit(
            "batch_settled",
            &BatchSettled {
                batch_id: U64(batch_id),
                status: rec.status,
                sub_batches: rec.dispatched,
                settled: ok,
                failed,
                results: &rec.settled,
            },
        );
        let status = rec.status;
        self.batches.insert(batch_id, rec);
        status
    }

    // ------------------------------------------------------------ owner

    /// A new `project_id` clears the pinned keys: the next `run_keys` re-bootstraps.
    pub fn set_project(&mut self, project_id: String, version_key: String) {
        self.assert_owner();
        assert_project(&project_id, &version_key);
        if project_id != self.project_id {
            self.result_key = None;
            self.matcher_key = None;
            self.venue_key = None;
            self.config_changed("keys_cleared", &project_id);
        }
        self.project_id = project_id;
        self.version_key = version_key;
        self.config_changed("project", (&self.project_id, &self.version_key));
    }

    pub fn set_result_key(&mut self, key: String) {
        self.assert_owner();
        self.result_key = Some(key_or_panic(&key));
        self.config_changed("result_key", &self.result_key);
    }

    /// `key: None` lets the next verified keys run supply the venue key.
    pub fn set_venue(&mut self, account: AccountId, key: Option<String>) {
        self.assert_owner();
        self.venue_account = account;
        self.venue_key = key.map(|k| key_or_panic(&k));
        self.config_changed("venue", (&self.venue_account, &self.venue_key));
    }

    pub fn add_market(&mut self, market: Market) {
        self.assert_owner();
        assert_market(&market);
        require!(self.market(&market.id).is_none(), "market exists");
        require!(self.markets.len() < MAX_MARKETS, "too many markets");
        self.markets.push(market);
        self.config_changed("market_added", self.markets.last());
    }

    /// Pending orders for a removed market are still sent; the matcher rejects them.
    pub fn remove_market(&mut self, id: String) {
        self.assert_owner();
        let before = self.markets.len();
        self.markets.retain(|m| m.id != id);
        require!(self.markets.len() < before, "unknown market");
        self.config_changed("market_removed", &id);
    }

    pub fn set_params(&mut self, params: Params) {
        self.assert_owner();
        assert_params(&params);
        self.params = params;
        self.config_changed("params", &self.params);
    }

    /// Minimum crank deposit. Track OutLayer's compute estimate for the
    /// pinned resource limits (0.004002 NEAR at the time of writing).
    pub fn set_run_deposit(&mut self, amount: U128) {
        self.assert_owner();
        require!(amount.0 > 0, "run deposit must be positive");
        self.run_deposit = amount.0;
        self.config_changed("run_deposit", amount);
    }

    // ------------------------------------------------------------ views

    pub fn get_config(&self) -> Config {
        Config {
            owner: self.owner.clone(),
            network: self.network.clone(),
            outlayer: self.outlayer.clone(),
            project_id: self.project_id.clone(),
            version_key: self.version_key.clone(),
            result_key: self.result_key.clone(),
            matcher_key: self.matcher_key.clone(),
            venue_account: self.venue_account.clone(),
            venue_key: self.venue_key.clone(),
            verifier: self.verifier.clone(),
            params: self.params.clone(),
            run_deposit: U128(self.run_deposit),
            run_gas: U64(run_gas(self.params.max_sub_batches).as_gas()),
            pending: self.pending.len() as u32,
            max_pending: MAX_PENDING as u32,
            next_seq: self.next_seq,
            next_batch_id: self.next_batch_id,
            in_flight: self.in_flight.clone(),
        }
    }

    pub fn get_markets(&self) -> Vec<Market> {
        self.markets.clone()
    }

    /// The matcher's X25519 key (hex) to seal orders to, once a verified keys run published it.
    pub fn get_matcher_key(&self) -> Option<String> {
        self.matcher_key.clone()
    }

    /// A deposit that always covers `submit_order` for an envelope of `bytes`.
    pub fn get_order_deposit(&self, bytes: u32) -> U128 {
        U128((bytes as u128 + ORDER_DEPOSIT_OVERHEAD as u128) * STORAGE_PRICE_PER_BYTE)
    }

    /// Pending orders in submission order, without ciphertexts.
    pub fn get_pending(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<OrderView> {
        let from = from_index.unwrap_or(0) as usize;
        let limit = limit.unwrap_or(MAX_PENDING as u32) as usize;
        self.pending
            .iter()
            .skip(from)
            .take(limit)
            .filter_map(|e| self.order_view(e.id, false))
            .collect()
    }

    /// One pending order, ciphertext included.
    pub fn get_order(&self, id: U64) -> Option<OrderView> {
        self.order_view(id.0, true)
    }

    pub fn get_batch(&self, id: U64) -> Option<BatchRecord> {
        self.batches.get(&id.0).cloned()
    }

    /// Batch records with id >= `from` (default: the latest `limit`), oldest first.
    pub fn get_batches(&self, from: Option<U64>, limit: Option<u32>) -> Vec<BatchRecord> {
        let limit = limit.unwrap_or(20).min(50) as u64;
        let from = from
            .map(|f| f.0)
            .unwrap_or(self.next_batch_id.saturating_sub(limit));
        (from..self.next_batch_id.min(from.saturating_add(limit)))
            .filter_map(|id| self.batches.get(&id).cloned())
            .collect()
    }

    pub fn get_stats(&self) -> Stats {
        Stats {
            markets: self.stats.clone(),
            totals: self.totals.clone(),
        }
    }
}

// ---------------------------------------------------------------- internals

impl Pool {
    fn assert_owner(&self) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
    }

    fn config_changed<T: Serialize>(&self, field: &str, value: T) {
        emit(
            "config_changed",
            &ConfigChanged {
                field,
                by: &env::predecessor_account_id(),
                value,
            },
        );
    }

    fn market(&self, id: &str) -> Option<&Market> {
        self.markets.iter().find(|m| m.id == id)
    }

    fn stats_mut(&mut self, market: &str) -> &mut MarketStats {
        let i = match self.stats.iter().position(|s| s.market == market) {
            Some(i) => i,
            None => {
                self.stats.push(MarketStats {
                    market: market.to_string(),
                    ..Default::default()
                });
                self.stats.len() - 1
            }
        };
        &mut self.stats[i]
    }

    fn guard_live(&self) -> bool {
        self.in_flight
            .as_ref()
            .is_some_and(|f| env::block_height() <= f.block + GUARD_TIMEOUT_BLOCKS)
    }

    fn in_flight_live_contains(&self, id: u64) -> bool {
        self.guard_live()
            && self
                .in_flight
                .as_ref()
                .is_some_and(|f| f.orders.contains(&id))
    }

    fn take_pending(&mut self, id: u64) -> Option<Order> {
        let i = self.pending.iter().position(|e| e.id == id)?;
        self.pending.remove(i);
        self.orders.remove(&id)
    }

    fn order_view(&self, id: u64, with_sealed: bool) -> Option<OrderView> {
        let o = self.orders.get(&id)?;
        Some(OrderView {
            id: U64(id),
            seq: id,
            market: o.market.clone(),
            submitter: o.submitter.clone(),
            bytes: o.sealed.len() as u32,
            deposit: U128(o.deposit),
            submitted_at: U64(o.submitted_at),
            in_flight: self.in_flight_live_contains(id),
            sealed_b64: with_sealed.then(|| B64.encode(&o.sealed)),
        })
    }

    /// Input bytes the attached deposit pays for.
    fn input_budget(&self) -> usize {
        let attached = env::attached_deposit().as_yoctonear();
        require!(
            attached >= self.run_deposit,
            format!(
                "attach at least {} yoctoNEAR for OutLayer",
                self.run_deposit
            )
        );
        let extra = (attached - self.run_deposit) / STORAGE_PRICE_PER_BYTE;
        if extra == 0 {
            SOFT_INPUT_BYTES
        } else {
            (OUTLAYER_FREE_INPUT as u128 + extra).min(MAX_INPUT_BYTES as u128) as usize
        }
    }

    /// The §3.1 input. For `batch`, takes pending orders in submission order
    /// while the input stays within `budget`, skipping any that do not fit.
    fn build_input(&self, op: &str, budget: usize) -> (Vec<u64>, String) {
        let mut doc = Input {
            v: 1,
            op,
            network: &self.network,
            verifier: &self.verifier,
            venue_account: &self.venue_account,
            now_ns: env::block_timestamp().to_string(),
            markets: &self.markets,
            params: &self.params,
            orders: Vec::new(),
        };
        let mut ids = Vec::new();
        if op == OP_BATCH {
            let mut size = serde_json::to_string(&doc)
                .unwrap_or_else(|_| env::abort())
                .len();
            for e in &self.pending {
                if ids.len() >= MAX_ORDERS_PER_BATCH {
                    break;
                }
                // Upper bound until the order is loaded: market ids are <= 32 bytes.
                let item = input_order_len(e.id, MAX_MARKET_ID_LEN, e.len as usize)
                    + usize::from(!ids.is_empty());
                if size + item > budget {
                    continue;
                }
                let o = self.orders.get(&e.id).unwrap_or_else(|| env::abort());
                size += item - (MAX_MARKET_ID_LEN - o.market.len());
                ids.push(e.id);
                doc.orders.push(InputOrder {
                    id: e.id.to_string(),
                    seq: e.id,
                    market: o.market.clone(),
                    sealed: B64.encode(&o.sealed),
                });
            }
        }
        (
            ids,
            serde_json::to_string(&doc).unwrap_or_else(|_| env::abort()),
        )
    }

    fn dispatch(&mut self, op: &str, ids: Vec<u64>, input: String) -> Promise {
        require!(!self.guard_live(), "a batch is already running");
        let k = self.params.max_sub_batches;
        let need = run_gas(k);
        require!(
            env::prepaid_gas() >= need,
            format!("attach at least {} TGas", need.as_tgas())
        );
        let deposit = env::attached_deposit();
        let required = self.run_deposit + input_storage_deposit(input.len());
        require!(
            deposit.as_yoctonear() >= required,
            format!("attach at least {required} yoctoNEAR for OutLayer")
        );

        // A run whose guard expired is superseded; its late result still lands.
        if let Some(old) = self.in_flight.take() {
            if let Some(r) = self.batches.get_mut(&old.batch_id) {
                if r.status == BatchStatus::Requested {
                    r.status = BatchStatus::Expired;
                }
            }
        }

        let batch_id = self.next_batch_id;
        self.next_batch_id += 1;
        if batch_id >= BATCH_RETENTION {
            self.batches.remove(&(batch_id - BATCH_RETENTION));
        }
        let cranker = env::predecessor_account_id();
        let orders: Vec<U64> = ids.iter().map(|i| U64(*i)).collect();
        emit(
            "batch_requested",
            &BatchRequested {
                batch_id: U64(batch_id),
                op,
                cranker: &cranker,
                orders: &orders,
                input_bytes: input.len() as u32,
                deposit: U128(deposit.as_yoctonear()),
            },
        );
        self.batches.insert(
            batch_id,
            BatchRecord {
                id: U64(batch_id),
                op: op.to_string(),
                cranker: cranker.clone(),
                orders,
                input_bytes: input.len() as u32,
                input_hash: hex(&env::sha256_array(input.as_bytes())),
                deposit: U128(deposit.as_yoctonear()),
                requested_block: env::block_height(),
                requested_at: U64(env::block_timestamp()),
                finished_at: None,
                status: BatchStatus::Requested,
                error: None,
                result_hash: None,
                ingested: Vec::new(),
                rejected: Vec::new(),
                deferred: Vec::new(),
                refunded: U128(0),
                markets: Vec::new(),
                book: None,
                sub_batches: 0,
                dispatched: 0,
                settled: Vec::new(),
                keys: None,
            },
        );
        self.in_flight = Some(InFlight {
            batch_id,
            op: op.to_string(),
            orders: ids.clone(),
            block: env::block_height(),
        });
        self.totals.batches += 1;

        let args = RequestArgs {
            source: Source {
                project: ProjectSource {
                    project_id: &self.project_id,
                    version_key: Some(&self.version_key),
                },
            },
            resource_limits: Limits {
                max_instructions: MAX_INSTRUCTIONS,
                max_memory_mb: MAX_MEMORY_MB,
                max_execution_seconds: MAX_EXECUTION_SECONDS,
            },
            input_data: &input,
            response_format: "Json",
            payer_account_id: &cranker,
        };
        let args = serde_json::to_vec(&args).unwrap_or_else(|_| env::abort());
        Promise::new(self.outlayer.clone())
            .function_call_weight(
                "request_execution",
                args,
                deposit,
                REQUEST_GAS,
                GasWeight(1),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(on_result_gas(k))
                    .with_unused_gas_weight(0)
                    .on_result(batch_id, ids),
            )
    }

    /// Verifies the callback payload and applies it. Returns the settlement
    /// promise, if any; `Err` carries a terminal status.
    fn verify_and_apply(
        &mut self,
        rec: &mut BatchRecord,
        order_ids: &[u64],
    ) -> Result<Option<Promise>, Terminal> {
        let fail = |s: BatchStatus, e: &str| Err((s, e.to_string()));
        let raw = match env::promise_result_checked(0, MAX_RESULT_BYTES) {
            Ok(raw) => raw,
            Err(PromiseError::Failed) => {
                // `request_execution` itself failed, so its deposit came back here.
                if rec.deposit.0 > 0 {
                    Promise::new(rec.cranker.clone())
                        .transfer(NearToken::from_yoctonear(rec.deposit.0))
                        .detach();
                }
                return fail(
                    BatchStatus::Failed,
                    "outlayer call failed; deposit returned",
                );
            }
            Err(_) => return fail(BatchStatus::Failed, "result too long"),
        };
        if raw.trim_ascii() == b"null" {
            return fail(BatchStatus::Failed, "matcher run failed");
        }
        let out: Output = match serde_json::from_slice(&raw) {
            Ok(o) => o,
            Err(_) => {
                return fail(
                    BatchStatus::InvalidResult,
                    "output is not {result, sig, key}",
                )
            }
        };
        let (Some(result), Some(sig)) = (out.result, out.sig) else {
            return match out.error {
                // The module could not sign (§3.4): a failed run, nothing to act on.
                Some(code) => fail(
                    BatchStatus::Failed,
                    &format!("matcher error: {}", clip(&code)),
                ),
                None => fail(
                    BatchStatus::InvalidResult,
                    "output is not {result, sig, key}",
                ),
            };
        };
        let digest = env::sha256_array(result.as_bytes());
        rec.result_hash = Some(hex(&digest));

        let Some(pinned) = self.result_key.as_deref().and_then(parse_ed25519_key) else {
            if rec.op == OP_KEYS {
                // Bootstrap: record what the matcher claims so the owner can
                // check it against the attestation and pin it. Trust nothing yet.
                return match serde_json::from_slice::<KeysResult>(result.as_bytes()) {
                    Ok(k) if k.error.is_none() => {
                        rec.keys = Some(k.report());
                        fail(
                            BatchStatus::Unpinned,
                            "no result key pinned; keys recorded unverified",
                        )
                    }
                    Ok(k) => fail(
                        BatchStatus::Failed,
                        &format!("matcher error: {}", clip(&k.error.unwrap_or_default())),
                    ),
                    Err(_) => fail(BatchStatus::InvalidResult, "malformed keys result"),
                };
            }
            return fail(BatchStatus::RejectedSignature, "no result key pinned");
        };
        if let Some(key) = &out.key {
            if parse_ed25519_key(key) != Some(pinned) {
                return fail(
                    BatchStatus::RejectedSignature,
                    "result signed for another key",
                );
            }
        }
        let Some(sig) = parse_ed25519_sig(&sig) else {
            return fail(BatchStatus::RejectedSignature, "malformed signature");
        };
        if !env::ed25519_verify(&sig, digest, &pinned) {
            return fail(BatchStatus::RejectedSignature, "signature does not verify");
        }

        if rec.op == OP_KEYS {
            self.apply_keys(rec, result.as_bytes())?;
            return Ok(None);
        }
        self.apply_batch(rec, order_ids, result.as_bytes())
    }

    fn apply_keys(&mut self, rec: &mut BatchRecord, result: &[u8]) -> Result<(), Terminal> {
        let invalid = |e: &str| Err((BatchStatus::InvalidResult, e.to_string()));
        let Ok(k) = serde_json::from_slice::<KeysResult>(result) else {
            return invalid("malformed keys result");
        };
        check_head(k.v, &k.op, &rec.op)?;
        if let Some(code) = &k.error {
            return Err((
                BatchStatus::Failed,
                format!("matcher error: {}", clip(code)),
            ));
        }
        rec.keys = Some(k.report());
        if !is_x25519_hex(&k.matcher_x25519) {
            return invalid("matcher_x25519 is not 32 bytes of hex");
        }
        let (Some(venue), Some(result_key)) = (
            normalize_ed25519_key(&k.venue),
            normalize_ed25519_key(&k.result_key),
        ) else {
            return invalid("venue or result_key is not an ed25519 key");
        };
        if Some(&result_key) != self.result_key.as_ref() {
            return invalid("result_key differs from the pinned key");
        }
        match &self.venue_key {
            Some(v) if *v != venue => return invalid("venue key differs from the pinned key"),
            Some(_) => {}
            None => self.venue_key = Some(venue.clone()),
        }
        let matcher = k.matcher_x25519.to_ascii_lowercase();
        emit(
            "keys_published",
            &KeysPublished {
                batch_id: rec.id,
                matcher_x25519: &matcher,
                venue: &venue,
                result_key: &result_key,
            },
        );
        self.matcher_key = Some(matcher);
        rec.status = BatchStatus::Completed;
        Ok(())
    }

    fn apply_batch(
        &mut self,
        rec: &mut BatchRecord,
        order_ids: &[u64],
        result: &[u8],
    ) -> Result<Option<Promise>, Terminal> {
        let res: BatchResult = serde_json::from_slice(result).map_err(|_| {
            (
                BatchStatus::InvalidResult,
                "malformed batch result".to_string(),
            )
        })?;
        check_head(res.v, &res.op, &rec.op)?;
        if res.input_sha256 != rec.input_hash {
            return Err((
                BatchStatus::InvalidResult,
                "result answers another input".to_string(),
            ));
        }
        if let Some(code) = &res.error {
            // A failed run leaves the book untouched; every order stays pending.
            return Err((
                BatchStatus::Failed,
                format!("matcher error: {}", clip(code)),
            ));
        }

        // Only orders this batch actually sent, and still pending, leave the queue.
        let mut refunds: Vec<(AccountId, u128)> = Vec::new();
        let mut add_refund = |submitter: AccountId, amount: u128| match refunds
            .iter_mut()
            .find(|(a, _)| *a == submitter)
        {
            Some((_, sum)) => *sum += amount,
            None => refunds.push((submitter, amount)),
        };
        for id in res.ingested.iter().filter_map(|s| s.parse::<u64>().ok()) {
            if order_ids.contains(&id) {
                if let Some(o) = self.take_pending(id) {
                    add_refund(o.submitter, o.deposit);
                    rec.ingested.push(U64(id));
                }
            }
        }
        for r in &res.rejected {
            let Ok(id) = r.id.parse::<u64>() else {
                continue;
            };
            if order_ids.contains(&id) {
                if let Some(o) = self.take_pending(id) {
                    add_refund(o.submitter, o.deposit);
                    let mut reason = clip(&r.reason);
                    reason.truncate(floor_char_boundary(&reason, MAX_REASON_LEN));
                    rec.rejected.push(Rejection {
                        id: U64(id),
                        reason,
                    });
                }
            }
        }
        rec.deferred = res
            .deferred
            .iter()
            .filter_map(|s| s.parse::<u64>().ok())
            .filter(|id| order_ids.contains(id))
            .map(U64)
            .collect();
        self.totals.orders_ingested += rec.ingested.len() as u64;
        self.totals.orders_rejected += rec.rejected.len() as u64;
        let mut total = 0u128;
        for (submitter, amount) in refunds {
            total += amount;
            if amount > 0 {
                Promise::new(submitter)
                    .transfer(NearToken::from_yoctonear(amount))
                    .detach();
            }
        }
        rec.refunded = U128(total);
        rec.markets = res
            .batches
            .into_iter()
            .take(MAX_MARKETS)
            .map(|s| s.into_summary())
            .collect();
        rec.book = res.book;

        let settle: Vec<_> = res.settle.into_iter().filter(|s| !s.is_empty()).collect();
        rec.sub_batches = settle.len() as u32;
        if settle.is_empty() {
            rec.status = BatchStatus::Completed;
            return Ok(None);
        }
        // Each sub-batch is atomic on its own, so sending fewer is safe.
        let per = SETTLE_GAS.as_gas() + RECEIPT_OVERHEAD_GAS.as_gas();
        let reserve = ON_SETTLED_GAS.as_gas() + 2 * RECEIPT_OVERHEAD_GAS.as_gas();
        let left = env::prepaid_gas()
            .as_gas()
            .saturating_sub(env::used_gas().as_gas());
        let affordable = (left.saturating_sub(reserve) / per) as usize;
        let n = settle
            .len()
            .min(self.params.max_sub_batches as usize)
            .min(affordable);
        rec.dispatched = n as u32;
        if n < settle.len() {
            rec.error = Some(format!(
                "{} of {} sub-batches not sent",
                settle.len() - n,
                settle.len()
            ));
        }
        if n == 0 {
            rec.status = BatchStatus::SettlementFailed;
            return Ok(None);
        }
        rec.status = BatchStatus::Settling;
        let mut joined: Option<Promise> = None;
        for payloads in settle.iter().take(n) {
            let p = Promise::new(self.verifier.clone()).function_call(
                "execute_intents",
                execute_intents_args(payloads),
                NearToken::from_yoctonear(0),
                SETTLE_GAS,
            );
            joined = Some(match joined {
                None => p,
                Some(j) => j.and(p),
            });
        }
        let joined = joined.unwrap_or_else(|| env::abort());
        Ok(Some(
            joined.then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_SETTLED_GAS)
                    .with_unused_gas_weight(0)
                    .on_settled(rec.id.0, n as u32),
            ),
        ))
    }

    fn emit_batch_result(&self, rec: &BatchRecord) {
        let markets = rec
            .markets
            .iter()
            .map(|m| MarketLine {
                market: &m.market,
                matched_lots: m.matched_lots,
                base_volume: m.base_volume,
                quote_volume: m.quote_volume,
                venue_fee: m.venue_fee,
                mid: &m.mid,
                skipped: &m.skipped,
            })
            .collect();
        emit(
            "batch_result",
            &BatchResultEvent {
                batch_id: rec.id,
                op: &rec.op,
                status: rec.status,
                error: &rec.error,
                result_hash: &rec.result_hash,
                ingested: &rec.ingested,
                rejected: &rec.rejected,
                deferred: &rec.deferred,
                refunded: rec.refunded,
                markets,
                book: &rec.book,
                sub_batches: rec.sub_batches,
                dispatched: rec.dispatched,
            },
        );
    }
}

// ---------------------------------------------------------------- request_execution args

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct RequestArgs<'a> {
    source: Source<'a>,
    resource_limits: Limits,
    input_data: &'a str,
    response_format: &'a str,
    payer_account_id: &'a AccountId,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct Source<'a> {
    #[serde(rename = "Project")]
    project: ProjectSource<'a>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct ProjectSource<'a> {
    project_id: &'a str,
    version_key: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct Limits {
    max_instructions: u64,
    max_memory_mb: u32,
    max_execution_seconds: u64,
}

// ---------------------------------------------------------------- validation

fn check_head(v: u32, op: &str, requested: &str) -> Result<(), Terminal> {
    if v != 1 || op != requested {
        return Err((
            BatchStatus::InvalidResult,
            "result v/op does not match the request".to_string(),
        ));
    }
    Ok(())
}

fn key_or_panic(key: &str) -> String {
    normalize_ed25519_key(key)
        .unwrap_or_else(|| env::panic_str("expected an ed25519:<base58> public key"))
}

fn assert_project(project_id: &str, version_key: &str) {
    require!(
        !project_id.is_empty() && project_id.len() <= 128,
        "bad project_id"
    );
    require!(
        version_key.len() == 64 && version_key.bytes().all(|b| b.is_ascii_hexdigit()),
        "version_key must be the 64-hex wasm hash"
    );
}

fn assert_params(p: &Params) {
    // The matcher's bounds (§3.4); max_sub_batches is capped lower by gas.
    require!(p.fee_bps <= 1_000, "fee_bps must be <= 1000");
    require!(
        (1..=10_000).contains(&p.max_spread_bps),
        "max_spread_bps must be 1..=10000"
    );
    require!(
        (1..=6).contains(&p.max_lots_per_side),
        "max_lots_per_side must be 1..=6"
    );
    require!(
        (1..=MAX_SUB_BATCHES_CAP).contains(&p.max_sub_batches),
        "max_sub_batches must be 1..=12"
    );
    require!(
        (1..=86_400).contains(&p.deadline_margin_s),
        "deadline_margin_s must be 1..=86400"
    );
    require!(
        p.quote_max_age_s <= 86_400,
        "quote_max_age_s must be <= 86400"
    );
}

fn assert_market(m: &Market) {
    require!(
        !m.id.is_empty()
            && m.id.len() <= MAX_MARKET_ID_LEN
            && m.id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b)),
        "market id must be 1-32 chars of [a-z0-9._-]"
    );
    for t in [&m.base, &m.quote, &m.reference.base, &m.reference.quote] {
        require!(
            !t.is_empty()
                && t.len() <= MAX_TOKEN_LEN
                && t.bytes()
                    .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\'),
            "bad token id"
        );
    }
    require!(
        m.base != m.quote && m.reference.base != m.reference.quote,
        "base and quote must differ"
    );
    require!(
        m.lot.0 > 0 && m.reference.probe_base.0 > 0 && m.reference.probe_quote.0 > 0,
        "amounts must be positive"
    );
    require!(m.lot.0 <= i128::MAX as u128 / 1_000, "lot too large");
}

fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}
