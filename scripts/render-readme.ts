/**
 * Renders the measured numbers in README.md from the files in results/, so no
 * figure in the README is typed by hand:
 *
 * - `numbers`: the public route's cost curve, from the newest baseline-*.json;
 * - `mainnet`: the first settled mainnet batch, from the oldest settled
 *   e2e-mainnet-*.json run record.
 *
 *   pnpm readme
 */
import { readdirSync, readFileSync, writeFileSync } from "node:fs";

const results = readdirSync("results").sort();
const read = <T>(f: string) => JSON.parse(readFileSync(`results/${f}`, "utf8")) as T;
const replaceBlock = (text: string, name: string, block: string) =>
  text.replace(new RegExp(`<!-- ${name}:start -->[\\s\\S]*?<!-- ${name}:end -->`), () => block);

// ---------------------------------------------------------------- route cost

type Row = { side: "sell" | "buy"; near: number; usd_at_mid: number; cost_bps: number | null; cost_usd: number | null; route: string };
const file = results.filter((f) => f.startsWith("baseline-")).at(-1);
if (!file) throw new Error("no baseline in results/; run pnpm baseline");
const base = read<{ measured_at: string; rows: Row[] }>(file);

const sells = base.rows.filter((r) => r.side === "sell");
const lastFilled = sells.filter((r) => r.cost_bps !== null).at(-1)!;
const firstEmpty = sells.find((r) => r.cost_bps === null);
const usd = (n: number) => `$${Math.round(n).toLocaleString("en-US")}`;
const roundTo = (n: number, step: number) => Math.round(n / step) * step;

const numbers = [
  "<!-- numbers:start -->",
  `On the public NEAR Intents route, selling **${lastFilled.near.toLocaleString("en-US")} NEAR (~${usd(roundTo(lastFilled.usd_at_mid, 1000))})** cost **${lastFilled.cost_bps!.toFixed(1)} bps (about ${usd(roundTo(lastFilled.cost_usd!, 100))})** against the midpoint${
    firstEmpty ? `, and from **${firstEmpty.near.toLocaleString("en-US")} NEAR** the solvers answered \`No liquidity available\`` : ""
  }. Blindbook crosses the same block at the midpoint for a venue fee of at most 5 bps, taken only out of price improvement. *(1Click dry quotes, measured ${base.measured_at.slice(0, 10)}, [\`results/${file}\`](results/${file}).)*`,
  "<!-- numbers:end -->",
].join("\n");

// ---------------------------------------------------------------- mainnet batch

interface E2eRun {
  pool: string;
  verifier: string;
  lots_filled: number;
  txs: { run_batch: string };
  batch: {
    id: string;
    cranker: string;
    status: string;
    markets: { lot_mid_quote: string; venue_fee: string }[];
  } | null;
  buyer: { base_delta: string };
  checks: {
    seller_got_at_least_limit: boolean;
    buyer_paid_at_most_limit: boolean;
    seller_effective_price: string;
    buyer_effective_price: string;
  };
}

const BASE_DECIMALS = 24; // wNEAR
const QUOTE_DECIMALS = 6; // USDC
const mainFile = results
  .filter((f) => f.startsWith("e2e-mainnet-"))
  .find((f) => read<E2eRun>(f).batch?.status === "settled");
if (!mainFile) throw new Error("no settled mainnet run in results/; run pnpm e2e:mainnet");
const run = read<E2eRun>(mainFile);
const b = run.batch!;
const m = b.markets[0];
/** Exact decimal value of an integer amount in base units (dividing a Number would print 0.09999999999999999). */
const units = (v: bigint, decimals: number) => {
  const s = v.toString().padStart(decimals + 1, "0");
  return Number(`${s.slice(0, -decimals)}.${s.slice(-decimals)}`);
};
const lotNear = units(BigInt(run.buyer.base_delta) / BigInt(run.lots_filled), BASE_DECIMALS);
const lotMid = Number(m.lot_mid_quote) / 10 ** QUOTE_DECIMALS / lotNear;
const fee = Number(m.venue_fee) / 10 ** QUOTE_DECIMALS;
const tx = run.txs.run_batch;
const inside = run.checks.seller_got_at_least_limit && run.checks.buyer_paid_at_most_limit;
const lots = run.lots_filled === 1 ? "one" : String(run.lots_filled);

const mainnet = [
  "<!-- mainnet:start -->",
  `**Live on NEAR mainnet.** Batch ${b.id} on \`${run.pool}\` crossed ${lots} ${lotNear} NEAR lot${run.lots_filled === 1 ? "" : "s"} a side at the signed 1Click midpoint. Both legs settled in one call on \`${run.verifier}\`: [\`${tx.slice(0, 8)}…\`](https://nearblocks.io/txns/${tx}). The seller got ${run.checks.seller_effective_price} USDC per NEAR and the buyer paid ${run.checks.buyer_effective_price}, around a lot mid of ${lotMid.toFixed(5)}${inside ? ", both inside their signed limits" : ""}. The venue fee was ${fee} USDC. *(Run record: [\`results/${mainFile}\`](results/${mainFile}).)* Re-check it from public sources:`,
  "",
  "```bash",
  `pnpm verify:batch ${b.id} --tx ${tx} --sender ${b.cranker} --mainnet`,
  "```",
  "<!-- mainnet:end -->",
].join("\n");

// ---------------------------------------------------------------- write

const readme = readFileSync("README.md", "utf8");
const next = replaceBlock(replaceBlock(readme, "numbers", numbers), "mainnet", mainnet);
writeFileSync("README.md", next);
console.log(
  next === readme
    ? "README numbers already current"
    : `README numbers rendered from results/${file} and results/${mainFile}`,
);
