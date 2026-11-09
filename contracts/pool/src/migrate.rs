//! One-off upgrade of the testnet pool deployed before PROTOCOL §3.4:
//! `Params` gained `quote_max_age_s`, batch records gained `input_hash` and
//! `deferred`, market summaries gained `sub_batches` and `dropped`.
//!
//! Built only with `--features migrate-v0`; the production wasm has no
//! migration code. Deploy the feature build with `migrate_v0` as the init
//! call, then redeploy the normal build without one.

use crate::*;

#[near(serializers = [borsh])]
struct OldParams {
    fee_bps: u32,
    max_spread_bps: u32,
    max_lots_per_side: u32,
    max_sub_batches: u32,
    deadline_margin_s: u32,
}

#[near(serializers = [borsh])]
struct OldMarketSummary {
    market: String,
    mid: Option<Mid>,
    lot_mid_quote: Option<U128>,
    matched_lots: u32,
    base_volume: U128,
    quote_volume: U128,
    rebated: U128,
    venue_fee: U128,
    quotes: Vec<String>,
    skipped: Option<String>,
}

#[near(serializers = [borsh])]
struct OldBatchRecord {
    id: U64,
    op: String,
    cranker: AccountId,
    orders: Vec<U64>,
    input_bytes: u32,
    deposit: U128,
    requested_block: u64,
    requested_at: U64,
    finished_at: Option<U64>,
    status: BatchStatus,
    error: Option<String>,
    result_hash: Option<String>,
    ingested: Vec<U64>,
    rejected: Vec<Rejection>,
    refunded: U128,
    markets: Vec<OldMarketSummary>,
    book: Option<BookCounts>,
    sub_batches: u32,
    dispatched: u32,
    settled: Vec<bool>,
    keys: Option<KeysReport>,
}

#[near(serializers = [borsh])]
struct OldPool {
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
    params: OldParams,
    run_deposit: u128,
    pending: Vec<PendingEntry>,
    orders: LookupMap<u64, Order>,
    next_seq: u64,
    next_batch_id: u64,
    batches: LookupMap<u64, OldBatchRecord>,
    in_flight: Option<InFlight>,
    stats: Vec<MarketStats>,
    totals: Totals,
}

#[near]
impl Pool {
    /// Rewrites the old state and every retained batch record in the new layout.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_v0(deadline_margin_s: u32, quote_max_age_s: u32) -> Self {
        let old: OldPool =
            env::state_read().unwrap_or_else(|| env::panic_str("no state to migrate"));
        let p = old.params;
        let params = Params {
            fee_bps: p.fee_bps,
            max_spread_bps: p.max_spread_bps,
            max_lots_per_side: p.max_lots_per_side,
            max_sub_batches: p.max_sub_batches,
            deadline_margin_s,
            quote_max_age_s,
        };
        assert_params(&params);
        let mut batches: LookupMap<u64, BatchRecord> = LookupMap::new(b"b");
        for id in old.next_batch_id.saturating_sub(BATCH_RETENTION)..old.next_batch_id {
            let Some(r) = old.batches.get(&id) else {
                continue;
            };
            let markets = r
                .markets
                .iter()
                .map(|m| MarketSummary {
                    market: m.market.clone(),
                    mid: m.mid.clone(),
                    lot_mid_quote: m.lot_mid_quote,
                    matched_lots: m.matched_lots,
                    base_volume: m.base_volume,
                    quote_volume: m.quote_volume,
                    rebated: m.rebated,
                    venue_fee: m.venue_fee,
                    quotes: m.quotes.clone(),
                    sub_batches: 0,
                    dropped: Vec::new(),
                    skipped: m.skipped.clone(),
                })
                .collect();
            // `set` writes without reading the old-layout value.
            batches.set(
                id,
                Some(BatchRecord {
                    id: r.id,
                    op: r.op.clone(),
                    cranker: r.cranker.clone(),
                    orders: r.orders.clone(),
                    input_bytes: r.input_bytes,
                    input_hash: String::new(),
                    deposit: r.deposit,
                    requested_block: r.requested_block,
                    requested_at: r.requested_at,
                    finished_at: r.finished_at,
                    status: r.status,
                    error: r.error.clone(),
                    result_hash: r.result_hash.clone(),
                    ingested: r.ingested.clone(),
                    rejected: r.rejected.clone(),
                    deferred: Vec::new(),
                    refunded: r.refunded,
                    markets,
                    book: r.book.clone(),
                    sub_batches: r.sub_batches,
                    dispatched: r.dispatched,
                    settled: r.settled.clone(),
                    keys: r.keys.clone(),
                }),
            );
        }
        self::emit_migrated(old.next_batch_id);
        Self {
            owner: old.owner,
            network: old.network,
            outlayer: old.outlayer,
            project_id: old.project_id,
            version_key: old.version_key,
            result_key: old.result_key,
            matcher_key: old.matcher_key,
            venue_account: old.venue_account,
            venue_key: old.venue_key,
            verifier: old.verifier,
            markets: old.markets,
            params,
            run_deposit: old.run_deposit,
            pending: old.pending,
            orders: old.orders,
            next_seq: old.next_seq,
            next_batch_id: old.next_batch_id,
            batches,
            in_flight: old.in_flight,
            stats: old.stats,
            totals: old.totals,
        }
    }
}

fn emit_migrated(records: u64) {
    emit(
        "config_changed",
        &ConfigChanged {
            field: "migrated_v0",
            by: &env::predecessor_account_id(),
            value: records,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    #[test]
    fn migrates_state_and_batch_records() {
        let pool: AccountId = "pool.near".parse().unwrap();
        testing_env!(VMContextBuilder::new()
            .current_account_id(pool.clone())
            .predecessor_account_id(pool)
            .build());
        let mut batches: LookupMap<u64, OldBatchRecord> = LookupMap::new(b"b");
        batches.insert(
            0,
            OldBatchRecord {
                id: U64(0),
                op: "batch".into(),
                cranker: "crank.near".parse().unwrap(),
                orders: vec![U64(0)],
                input_bytes: 600,
                deposit: U128(5),
                requested_block: 1,
                requested_at: U64(2),
                finished_at: Some(U64(3)),
                status: BatchStatus::Settled,
                error: None,
                result_hash: Some("ab".into()),
                ingested: vec![U64(0)],
                rejected: vec![],
                refunded: U128(9),
                markets: vec![OldMarketSummary {
                    market: "m".into(),
                    mid: None,
                    lot_mid_quote: None,
                    matched_lots: 1,
                    base_volume: U128(1),
                    quote_volume: U128(2),
                    rebated: U128(0),
                    venue_fee: U128(1),
                    quotes: vec![],
                    skipped: None,
                }],
                book: None,
                sub_batches: 1,
                dispatched: 1,
                settled: vec![true],
                keys: None,
            },
        );
        batches.flush();
        let old = OldPool {
            owner: "owner.near".parse().unwrap(),
            network: "testnet".into(),
            outlayer: "outlayer.testnet".parse().unwrap(),
            project_id: "p/x".into(),
            version_key: "a".repeat(64),
            result_key: None,
            matcher_key: None,
            venue_account: "venue.near".parse().unwrap(),
            venue_key: None,
            verifier: "intents.near".parse().unwrap(),
            markets: vec![],
            params: OldParams {
                fee_bps: 5,
                max_spread_bps: 150,
                max_lots_per_side: 6,
                max_sub_batches: 4,
                deadline_margin_s: 90,
            },
            run_deposit: 7,
            pending: vec![],
            orders: LookupMap::new(b"o"),
            next_seq: 3,
            next_batch_id: 1,
            batches,
            in_flight: None,
            stats: vec![],
            totals: Totals::default(),
        };
        env::state_write(&old);

        let p = Pool::migrate_v0(150, 120);
        assert_eq!(p.params.deadline_margin_s, 150);
        assert_eq!(p.params.quote_max_age_s, 120);
        assert_eq!((p.next_seq, p.run_deposit), (3, 7));
        let r = p.get_batch(U64(0)).unwrap();
        assert_eq!(r.status, BatchStatus::Settled);
        assert_eq!(r.markets[0].quote_volume, U128(2));
        assert_eq!(r.input_hash, "");
    }
}
