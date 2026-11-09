/**
 * Renders the measured numbers in README.md from results/baseline-*.json, so
 * no figure in the README is typed by hand.
 *
 *   pnpm readme
 */
import { readdirSync, readFileSync, writeFileSync } from "node:fs";

type Row = { side: "sell" | "buy"; near: number; usd_at_mid: number; cost_bps: number | null; cost_usd: number | null; route: string };
const file = readdirSync("results").filter((f) => f.startsWith("baseline-")).sort().at(-1);
if (!file) throw new Error("no baseline in results/; run pnpm baseline");
const base = JSON.parse(readFileSync(`results/${file}`, "utf8")) as { measured_at: string; rows: Row[] };

const sells = base.rows.filter((r) => r.side === "sell");
const lastFilled = sells.filter((r) => r.cost_bps !== null).at(-1)!;
const firstEmpty = sells.find((r) => r.cost_bps === null);
const usd = (n: number) => `$${Math.round(n).toLocaleString("en-US")}`;
const roundTo = (n: number, step: number) => Math.round(n / step) * step;

const block = [
  "<!-- numbers:start -->",
  `On the public NEAR Intents route, selling **${lastFilled.near.toLocaleString("en-US")} NEAR (~${usd(roundTo(lastFilled.usd_at_mid, 1000))})** cost **${lastFilled.cost_bps!.toFixed(1)} bps (about ${usd(roundTo(lastFilled.cost_usd!, 100))})** against the midpoint${
    firstEmpty ? `, and from **${firstEmpty.near.toLocaleString("en-US")} NEAR** the solvers answered \`No liquidity available\`` : ""
  }. Blindbook crosses the same block at the midpoint for a venue fee of at most 5 bps, taken only out of price improvement. *(1Click dry quotes, measured ${base.measured_at.slice(0, 10)}, [\`results/${file}\`](results/${file}).)*`,
  "<!-- numbers:end -->",
].join("\n");

const readme = readFileSync("README.md", "utf8");
const next = readme.replace(/<!-- numbers:start -->[\s\S]*?<!-- numbers:end -->/, block);
writeFileSync("README.md", next);
console.log(next === readme ? "README numbers already current" : `README numbers rendered from results/${file}`);
