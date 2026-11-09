//! State and view types. JSON shapes of `Market` and `Params` are part of the
//! matcher input (PROTOCOL §3.1), so their field order is load-bearing.

use near_sdk::json_types::{U128, U64};
use near_sdk::{near, AccountId};

/// Assets the matcher prices a market against (1Click quotes). On testnet the
/// test tokens map to their mainnet equivalents here.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub base: String,
    pub quote: String,
    /// Bid probe: sell this much `base`.
    pub probe_base: U128,
    /// Ask probe: buy with this much `quote`.
    pub probe_quote: U128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Market {
    pub id: String,
    pub base: String,
    pub quote: String,
    /// Lot size in base units.
    pub lot: U128,
    pub reference: Reference,
}

/// Matcher parameters, forwarded verbatim in every input.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Params {
    pub fee_bps: u32,
    pub max_spread_bps: u32,
    pub max_lots_per_side: u32,
    pub max_sub_batches: u32,
    pub deadline_margin_s: u32,
    /// Max age of a 1Click quote against the enclave clock; 0 disables the check (§3.4).
    pub quote_max_age_s: u32,
}

/// A sealed order waiting for the matcher. The id is the map key (= `seq`).
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub market: String,
    pub submitter: AccountId,
    /// The envelope bytes (base64-decoded).
    pub sealed: Vec<u8>,
    /// Storage deposit held for this order, refunded in full when it leaves.
    pub deposit: u128,
    /// Block timestamp, ns.
    pub submitted_at: u64,
}

/// Index entry for a pending order. The length lets `run_batch` size its
/// input without reading ciphertexts it will not send.
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingEntry {
    pub id: u64,
    pub len: u32,
}

/// Order as shown by views. `sealed_b64` is set by `get_order` only.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderView {
    pub id: U64,
    pub seq: u64,
    pub market: String,
    pub submitter: AccountId,
    pub bytes: u32,
    pub deposit: U128,
    pub submitted_at: U64,
    /// Sent to the matcher in the batch now running.
    pub in_flight: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sealed_b64: Option<String>,
}

/// The OutLayer run currently awaiting its callback.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InFlight {
    pub batch_id: u64,
    pub op: String,
    pub orders: Vec<u64>,
    pub block: u64,
}

#[near(serializers = [borsh, json])]
#[serde(rename_all = "snake_case")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchStatus {
    /// Sent to OutLayer, waiting for the result.
    Requested,
    /// The guard timed out and another run took over. A late result still lands.
    Expired,
    /// OutLayer returned `null` (module failure, timeout, denial) or the call failed.
    Failed,
    /// The result was not signed by the pinned result key.
    RejectedSignature,
    /// Signed, but malformed or inconsistent with the request.
    InvalidResult,
    /// A keys result arrived before any result key was pinned. Recorded, not trusted.
    Unpinned,
    /// Verified; nothing to settle (or a keys result applied).
    Completed,
    /// Verified; `execute_intents` calls are in flight.
    Settling,
    Settled,
    PartiallySettled,
    SettlementFailed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub id: U64,
    pub reason: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mid {
    pub quote: String,
    pub base: String,
}

/// One market's outcome in a batch, as reported by the matcher.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketSummary {
    pub market: String,
    pub mid: Option<Mid>,
    pub lot_mid_quote: Option<U128>,
    pub matched_lots: u32,
    pub base_volume: U128,
    pub quote_volume: U128,
    pub rebated: U128,
    pub venue_fee: U128,
    /// 1Click quote digests the mid was derived from.
    pub quotes: Vec<String>,
    /// This market's sub-batches in `settle`.
    pub sub_batches: u32,
    /// One reason per sub-batch that crossed but was left out.
    pub dropped: Vec<String>,
    pub skipped: Option<String>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookCounts {
    pub orders: u32,
    pub lots: u32,
}

/// Keys reported by a `keys` run.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysReport {
    pub matcher_x25519: String,
    pub venue: String,
    pub result_key: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchRecord {
    pub id: U64,
    pub op: String,
    pub cranker: AccountId,
    /// Orders sent in the input.
    pub orders: Vec<U64>,
    pub input_bytes: u32,
    /// Hex sha256 of the input sent; a batch result must echo it (`input_sha256`).
    pub input_hash: String,
    /// Deposit forwarded to OutLayer (unused part refunded to the cranker by OutLayer).
    pub deposit: U128,
    pub requested_block: u64,
    pub requested_at: U64,
    pub finished_at: Option<U64>,
    pub status: BatchStatus,
    pub error: Option<String>,
    /// Hex sha256 of the signed `result` string: the bytes the matcher signed.
    pub result_hash: Option<String>,
    pub ingested: Vec<U64>,
    pub rejected: Vec<Rejection>,
    /// Sent but neither ingested nor rejected: still pending.
    pub deferred: Vec<U64>,
    /// Storage deposits returned to submitters.
    pub refunded: U128,
    pub markets: Vec<MarketSummary>,
    pub book: Option<BookCounts>,
    /// Sub-batches in the result / actually sent to the verifier.
    pub sub_batches: u32,
    pub dispatched: u32,
    /// Per dispatched sub-batch: did `execute_intents` succeed.
    pub settled: Vec<bool>,
    pub keys: Option<KeysReport>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketStats {
    pub market: String,
    /// Batches in which all of this market's sub-batches settled.
    pub batches: u64,
    pub crossed_lots: u64,
    pub base_volume: U128,
    pub quote_volume: U128,
    pub venue_fees: U128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub batches: u64,
    /// Runs that ended failed, rejected or invalid.
    pub failed: u64,
    pub orders_ingested: u64,
    pub orders_rejected: u64,
    pub sub_batches_settled: u64,
    pub sub_batches_failed: u64,
}

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    pub markets: Vec<MarketStats>,
    pub totals: Totals,
}

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub owner: AccountId,
    pub network: String,
    pub outlayer: AccountId,
    pub project_id: String,
    pub version_key: String,
    pub result_key: Option<String>,
    pub matcher_key: Option<String>,
    pub venue_account: AccountId,
    pub venue_key: Option<String>,
    pub verifier: AccountId,
    pub params: Params,
    /// Minimum deposit for `run_batch` / `run_keys` (the OutLayer estimate).
    pub run_deposit: U128,
    /// Prepaid gas `run_batch` / `run_keys` require, in gas units.
    pub run_gas: U64,
    pub pending: u32,
    pub max_pending: u32,
    pub next_seq: u64,
    pub next_batch_id: u64,
    pub in_flight: Option<InFlight>,
}
