import type { Side } from "@blindbook/sdk";

/** Response shapes of the routes under /api, as the browser receives them. */

export interface PoolParams {
  fee_bps: number;
  max_spread_bps: number;
  max_lots_per_side: number;
  max_sub_batches: number;
  deadline_margin_s: number;
  quote_max_age_s?: number;
}

export interface InFlightView {
  batch_id: number;
  op: string;
  orders: number[];
  block: number;
}

export interface PoolConfigView {
  owner: string;
  network: string;
  project_id: string;
  version_key: string;
  result_key: string | null;
  matcher_key: string | null;
  venue_account: string;
  venue_key: string | null;
  verifier: string;
  params: PoolParams;
  run_deposit: string;
  run_gas: string;
  pending: number;
  max_pending?: number;
  next_batch_id: number;
  in_flight: InFlightView | null;
}

export interface PoolStatsView {
  markets: {
    market: string;
    batches: number;
    crossed_lots: number;
    base_volume: string;
    quote_volume: string;
    venue_fees: string;
  }[];
  totals: {
    batches: number;
    failed: number;
    orders_ingested: number;
    orders_rejected: number;
    sub_batches_settled: number;
    sub_batches_failed: number;
  };
}

export interface PoolResponse {
  network: string;
  pool: string;
  verifier: string;
  config: PoolConfigView;
  stats: PoolStatsView;
}

export type BatchStatus =
  | "requested"
  | "expired"
  | "failed"
  | "rejected_signature"
  | "invalid_result"
  | "unpinned"
  | "completed"
  | "settling"
  | "settled"
  | "partially_settled"
  | "settlement_failed";

export interface BatchMarketView {
  market: string;
  mid: { quote: string; base: string } | null;
  lot_mid_quote: string;
  matched_lots: number;
  base_volume: string;
  quote_volume: string;
  rebated: string;
  venue_fee: string;
  quotes: string[];
  sub_batches: number;
  dropped?: string[];
  skipped: string | null;
}

/** A batch's public, post-trade record. It never carries a resting order. */
export interface BatchView {
  id: string;
  op: string;
  cranker: string;
  orders: string[];
  input_bytes: number;
  input_hash: string;
  requested_block: number;
  requested_at: string;
  finished_at: string | null;
  status: BatchStatus;
  error: string | null;
  result_hash: string | null;
  ingested: string[];
  rejected: { id: string; reason: string }[];
  deferred?: string[];
  markets: BatchMarketView[];
  book: { orders: number; lots: number } | null;
  sub_batches: number;
  dispatched: number;
  settled: boolean[];
}

export interface Check {
  id: string;
  label: string;
  ok: boolean | null;
  detail: string;
  link?: string;
}

export interface VerifyResponse {
  batch: BatchView | null;
  checks: Check[];
  request_id: string | null;
}

export interface RouteCostResponse {
  side: Side;
  near: number;
  mid: number;
  value_at_mid_usd: number;
  route_cost_bps: number | null;
  route: string;
  measured_at: string;
}

export interface BalancesResponse {
  account: string;
  balances: Record<string, string>;
  keyRegistered: boolean | null;
  verifier: string;
}

export type FaucetResponse =
  | { ok: true; account: string; base: string; quote: string; txs: { base: string; quote: string } }
  | { ok: false; error: string };

export interface DemoResponse {
  ok: boolean;
  side: Side;
  reason?: string;
  mid?: number;
  limit?: string;
  lot?: string;
  txs?: { house: string; order: string; crank: string };
  explorer?: string;
  batch?: BatchView | null;
  effective?: string | null;
}

export interface RelaySubmitResponse {
  ok: boolean;
  orderId: string | null;
  tx: string;
  error?: string;
}

export interface RelayCancelResponse {
  ok: boolean;
  tx: string;
  error?: string;
}

export interface CrankResponse {
  ok: boolean;
  tx?: string;
  batchId?: number;
  cranker?: string;
  error?: string;
}

export interface BrokerInfo {
  available: boolean;
  model: string;
}

export interface LotPlan {
  side: Side;
  lots: number;
  limit_price: string;
  ladder_step_bps: number;
  min_lots: number;
  ttl_minutes: number;
  rationale: string;
}

/** The measured public-route cost curve from the repo's results/ baseline, per side. */
export interface RouteCurve {
  measuredAt: string;
  mid: number;
  rows: Record<Side, { near: number; bps: number | null }[]>;
}

export type BrokerResponse =
  | { ok: true; plan: LotPlan; model: string; checks: Check[] }
  | { ok: false; error: string; checks?: Check[] };
