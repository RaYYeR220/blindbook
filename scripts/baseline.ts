/**
 * Measures what the public NEAR Intents route charges for blocks of
 * increasing size and writes results/baseline-<date>.json.
 *
 *   pnpm baseline
 *
 * Every number in the README is rendered from these files.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { MAINNET, priceOfLot, referenceMid, routeCost } from "../packages/sdk/src/index.js";

const market = MAINNET.markets[0];
const NEAR = 10n ** 24n;
const sizes = [10n, 100n, 1_000n, 10_000n, 50_000n, 100_000n, 150_000n, 200_000n, 250_000n, 500_000n];

const mid = await referenceMid(market, 10n * NEAR, 36_000_000n);
const rows = [];
for (const side of ["sell", "buy"] as const) {
  for (const n of sizes) {
    const r = await routeCost(market, side, n * NEAR, mid);
    rows.push({
      side,
      near: Number(n),
      usd_at_mid: Number((r.midQuote * 100n) / 10n ** 6n) / 100,
      cost_bps: r.costBps,
      cost_usd: r.costBps === null ? null : Math.round((Number(r.midQuote) / 1e6) * (r.costBps / 10_000) * 100) / 100,
      route: r.costBps === null ? `unavailable: ${r.failure}` : "ok",
    });
    console.log(side.padEnd(4), String(n).padStart(7), "NEAR", r.costBps === null ? `-> ${r.failure}` : `-> ${r.costBps.toFixed(1)} bps`);
    await new Promise((res) => setTimeout(res, 400));
  }
}
const at = new Date().toISOString();
const out = {
  measured_at: at,
  source: "1Click dry quotes, EXACT_INPUT, INTENTS deposit and recipient",
  market: market.id,
  reference: { bid: mid.bid, ask: mid.ask, mid: Number(priceOfLot({ ...market, lot: mid.base }, mid.quote, 6)) },
  rows,
};
mkdirSync("results", { recursive: true });
const file = `results/baseline-${at.slice(0, 10)}.json`;
writeFileSync(file, `${JSON.stringify(out, null, 2)}\n`);
console.log(`wrote ${file}`);
