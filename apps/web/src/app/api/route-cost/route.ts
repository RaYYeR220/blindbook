import { MAINNET, referenceMid, routeCost } from "@blindbook/sdk";
import type { NextRequest } from "next/server";

/** What a block of NEAR would cost on the public 1Click route right now, vs the midpoint. */
export async function GET(request: NextRequest) {
  const side = request.nextUrl.searchParams.get("side") === "buy" ? "buy" : "sell";
  const near = Math.min(1_000_000, Math.max(0.1, Number(request.nextUrl.searchParams.get("near") ?? 100_000)));
  const market = MAINNET.markets[0];
  const mid = await referenceMid(market, 10n * 10n ** 24n, 36_000_000n);
  const r = await routeCost(market, side, BigInt(Math.round(near * 1e6)) * 10n ** 18n, mid);
  return Response.json({
    side,
    near,
    mid: (mid.bid + mid.ask) / 2,
    value_at_mid_usd: Number(r.midQuote) / 1e6,
    route_cost_bps: r.costBps,
    route: r.costBps === null ? `unavailable: ${r.failure}` : "available",
    measured_at: new Date().toISOString(),
  });
}
