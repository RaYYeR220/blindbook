import "server-only";
import { rpcView } from "@blindbook/sdk";
import { network } from "./env";

type Entry = { at: number; value: unknown };
const cache = new Map<string, Entry>();

/** Read a pool view, cached for `ttlMs` per process. */
export async function poolView<T>(method: string, args: object = {}, ttlMs = 8_000): Promise<T> {
  const key = `${method}:${JSON.stringify(args)}`;
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < ttlMs) return hit.value as T;
  const value = await rpcView<T>(network.rpc, network.pool, method, args);
  cache.set(key, { at: Date.now(), value });
  return value;
}

export function forget(prefix?: string) {
  for (const k of cache.keys()) if (!prefix || k.startsWith(prefix)) cache.delete(k);
}

export interface BatchMarket {
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
  dropped: unknown[];
  skipped: string | null;
}

export interface BatchRecord {
  id: string;
  op: string;
  cranker: string;
  orders: string[];
  input_bytes: number;
  input_hash: string;
  requested_block: number;
  requested_at: string;
  finished_at: string | null;
  status: string;
  error: string | null;
  result_hash: string | null;
  ingested: string[];
  rejected: { id: string; reason: string }[];
  deferred: string[];
  markets: BatchMarket[];
  book: { orders: number; lots: number } | null;
  sub_batches: number;
  dispatched: number;
  settled: boolean[];
}

export interface PoolStats {
  markets: { market: string; batches: number; crossed_lots: number; base_volume: string; quote_volume: string; venue_fees: string }[];
  totals: { batches: number; failed: number; orders_ingested: number; orders_rejected: number; sub_batches_settled: number; sub_batches_failed: number };
}
