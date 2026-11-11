import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { BASELINE_FILE, type Deployment, type PinnedBatch, verifyCommand } from "./proof";

/**
 * Typed readers for the measurement files in the repo's `results/` folder.
 * They run during prerender (synchronous reads), so the numbers are baked into
 * the static page and nothing on it is typed by hand.
 */

/** The repo's `results/` folder. The app builds and runs from apps/web. */
const RESULTS = join(process.cwd(), "..", "..", "results");

const readJson = <T>(file: string): T => JSON.parse(readFileSync(join(RESULTS, file), "utf8")) as T;

/** Exact decimal value of an integer amount in base units (dividing a Number would give 0.09999999999999999). */
const units = (v: bigint, decimals: number) => {
  const s = v.toString().padStart(decimals + 1, "0");
  return Number(`${s.slice(0, -decimals)}.${s.slice(-decimals)}`);
};

// ---------------------------------------------------------------- baseline

export type Side = "sell" | "buy";

interface BaselineFile {
  measured_at: string;
  source: string;
  market: string;
  reference: { bid: number; ask: number; mid: number };
  rows: {
    side: Side;
    near: number;
    usd_at_mid: number;
    cost_bps: number | null;
    cost_usd: number | null;
    route: string;
  }[];
}

export interface RouteRow {
  side: Side;
  near: number;
  usdAtMid: number;
  /** null when the route returned no quote */
  costBps: number | null;
  costUsd: number | null;
  /** the route's own message when it returned no quote */
  unavailable: string | null;
}

export interface Baseline {
  measuredAt: string;
  /** "wNEAR/USDC" */
  pair: string;
  exactInput: boolean;
  mid: number;
  rows: Record<Side, RouteRow[]>;
}

const pairName = (market: string) =>
  market
    .split("-")
    .map((t) => (t === "wnear" ? "wNEAR" : t.toUpperCase()))
    .join("/");

export function loadBaseline(): Baseline {
  const f = readJson<BaselineFile>(BASELINE_FILE);
  const rows: Record<Side, RouteRow[]> = { sell: [], buy: [] };
  for (const r of f.rows) {
    const failed = r.cost_bps == null;
    rows[r.side].push({
      side: r.side,
      near: r.near,
      usdAtMid: r.usd_at_mid,
      costBps: r.cost_bps,
      costUsd: r.cost_usd,
      unavailable: failed ? r.route.replace(/^unavailable:\s*/, "") : null,
    });
  }
  for (const side of ["sell", "buy"] as const) rows[side].sort((a, b) => a.near - b.near);
  return {
    measuredAt: f.measured_at,
    pair: pairName(f.market),
    exactInput: /EXACT_INPUT/.test(f.source),
    mid: f.reference.mid,
    rows,
  };
}

/** The biggest size the route still quoted, and the first size it refused. */
export function routeEdge(rows: RouteRow[]) {
  const quoted = rows.filter((r) => r.costBps != null);
  const largest = quoted[quoted.length - 1];
  const refused = rows.find((r) => r.costBps == null && r.near > (largest?.near ?? 0)) ?? null;
  if (!largest) throw new Error("Baseline has no quoted rows");
  return { largest: largest as RouteRow & { costBps: number; costUsd: number }, refused };
}

// ---------------------------------------------------------------- e2e run

interface E2eFile {
  at: string;
  network: string;
  pool: string;
  verifier: string;
  reference_mid: number;
  lots: number;
  lots_filled: number;
  txs: { sell: string; buy: string; run_batch: string };
  batch: {
    id: string;
    result_hash: string | null;
    markets: {
      mid: { quote: string; base: string } | null;
      /** the mid for one lot, in quote units, as the batch settled it */
      lot_mid_quote?: string;
      venue_fee?: string;
      quotes: string[];
    }[];
  } | null;
  seller: { account: string; base_delta: string; quote_delta: string };
  buyer: { account: string; base_delta: string; quote_delta: string };
  checks: {
    seller_got_at_least_limit: boolean;
    buyer_paid_at_most_limit: boolean;
    seller_effective_price: string | null;
    buyer_effective_price: string | null;
  };
}

export interface BatchProof {
  id: number;
  net: Deployment;
  at: string;
  pool: string;
  /** lots filled on each side, and the size of one lot in NEAR */
  lots: number;
  lotNear: number;
  mid: number;
  sellLimit: number;
  buyLimit: number;
  sellerPrice: number;
  buyerPrice: number;
  /** distance of each fill from the mid, in basis points (positive) */
  sellerBps: number;
  buyerBps: number;
  withinLimits: boolean;
  /** decimals to show limits and fills with: 5 when the signed limits are known exactly, else 4 */
  priceDigits: number;
  /** the venue's fee for the whole batch, in USDC, when the run record has it */
  venueFee: number | null;
  txs: { sell: string; buy: string; crank: string };
  quoteDigests: readonly string[];
  resultHash: string;
  outlayerRequest: number;
  accounts: { seller: string; buyer: string; cranker: string };
  /** `pnpm verify:batch …` for this batch */
  verify: string;
}

/**
 * The e2e run for a batch pinned in proof.ts. Files are scanned newest first,
 * so a later run of the same batch wins; a run of some other batch is skipped
 * until proof.ts points at it, which keeps the ids on the page in step with
 * the numbers.
 */
export function loadBatchProof(batch: PinnedBatch): BatchProof {
  const net = batch.net;
  const files = readdirSync(RESULTS)
    .filter((f) => f.startsWith(`e2e-${net.id}-`) && f.endsWith(".json"))
    .sort()
    .reverse();
  if (!files.length) throw new Error(`No results/e2e-${net.id}-*.json run found`);
  const runs = files.map((f) => readJson<E2eFile>(f));
  const run = runs.find((r) => r.txs.run_batch === batch.crankTx);
  if (!run) {
    throw new Error(
      `No e2e run in results/ cranked ${batch.crankTx}. Point the batch in src/lib/content/proof.ts at the run to show.`,
    );
  }

  const sellerPrice = Number(run.checks.seller_effective_price);
  const buyerPrice = Number(run.checks.buyer_effective_price);
  if (!Number.isFinite(sellerPrice) || !Number.isFinite(buyerPrice)) {
    throw new Error(`e2e run ${run.at} has no fills`);
  }

  const lots = run.lots_filled;
  const lotNear = units(BigInt(run.buyer.base_delta) / BigInt(lots), net.baseDecimals);
  const quoteUnit = 10 ** net.quoteDecimals;

  // The pool's record carries the mid the lot settled at; older runs lack it,
  // and both fills sit the same fee away from the mid on either side, so their
  // average is the mid.
  const market = run.batch?.markets[0];
  const recordedMid = market?.lot_mid_quote
    ? Number(market.lot_mid_quote) / quoteUnit / lotNear
    : market?.mid
      ? (Number(market.mid.quote) / Number(market.mid.base)) * 10 ** (net.baseDecimals - net.quoteDecimals)
      : null;
  const mid = recordedMid ?? (sellerPrice + buyerPrice) / 2;
  const band = batch.limitBandBps / 10_000;
  const quoteDigests = market?.quotes.length ? market.quotes : batch.quoteDigests;
  const resultHash = run.batch?.result_hash ?? batch.resultHash;
  if (!quoteDigests || !resultHash) throw new Error(`e2e run ${run.at} has no batch record; pin its digests in proof.ts`);

  return {
    id: run.batch ? Number(run.batch.id) : batch.id,
    net,
    at: run.at,
    pool: run.pool,
    lots,
    lotNear,
    mid,
    sellLimit: batch.signedLimits?.sell ?? run.reference_mid * (1 - band),
    buyLimit: batch.signedLimits?.buy ?? run.reference_mid * (1 + band),
    sellerPrice,
    buyerPrice,
    sellerBps: ((mid - sellerPrice) / mid) * 10_000,
    buyerBps: ((buyerPrice - mid) / mid) * 10_000,
    withinLimits: run.checks.seller_got_at_least_limit && run.checks.buyer_paid_at_most_limit,
    priceDigits: batch.signedLimits ? 5 : 4,
    venueFee: market?.venue_fee ? Number(market.venue_fee) / quoteUnit : null,
    txs: { sell: run.txs.sell, buy: run.txs.buy, crank: run.txs.run_batch },
    quoteDigests,
    resultHash,
    outlayerRequest: batch.outlayerRequest,
    accounts: { seller: run.seller.account, buyer: run.buyer.account, cranker: batch.cranker },
    verify: verifyCommand(batch),
  };
}
