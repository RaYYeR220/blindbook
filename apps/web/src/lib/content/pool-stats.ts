import { cacheLife } from "next/cache";
import { NETWORK } from "./proof";

export interface PoolStatsSummary {
  batches: number;
  crossedLots: number;
  ordersIngested: number;
}

interface StatsView {
  markets: { crossed_lots: number }[];
  totals: { batches: number; orders_ingested: number };
}

/**
 * The pool's `get_stats` view, read over NEAR RPC. Cached for a few minutes and
 * refreshed in the background; returns null when the RPC is slow or down so the
 * page falls back to its static numbers.
 */
export async function getPoolStats(): Promise<PoolStatsSummary | null> {
  "use cache";
  cacheLife("minutes");
  try {
    const res = await fetch(NETWORK.rpc, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: "landing",
        method: "query",
        params: {
          request_type: "call_function",
          finality: "final",
          account_id: NETWORK.pool,
          method_name: "get_stats",
          args_base64: Buffer.from("{}").toString("base64"),
        },
      }),
      signal: AbortSignal.timeout(4000),
    });
    if (!res.ok) return null;
    const body = (await res.json()) as { result?: { result?: number[] } };
    const raw = body.result?.result;
    if (!raw) return null;
    const stats = JSON.parse(Buffer.from(raw).toString("utf8")) as StatsView;
    return {
      batches: stats.totals.batches,
      crossedLots: stats.markets.reduce((n, m) => n + m.crossed_lots, 0),
      ordersIngested: stats.totals.orders_ingested,
    };
  } catch {
    return null;
  }
}
