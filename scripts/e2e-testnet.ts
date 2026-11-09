/**
 * End-to-end on testnet: two traders seal opposite orders, a third account
 * cranks one batch, the enclave crosses them at the live 1Click midpoint and
 * the pool settles both legs on the NEAR Intents verifier.
 *
 *   pnpm e2e:testnet
 *
 * Keys are read from ~/.near-credentials/testnet/<account>.json. Override the
 * accounts with SELLER, BUYER and CRANKER. Writes results/e2e-testnet-<ts>.json.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { Blindbook } from "../packages/client/src/index.js";
import { MAINNET, priceOfLot, referenceMid, TESTNET } from "../packages/sdk/src/index.js";

const SELLER = process.env.SELLER ?? "house.blindbook-dev.testnet";
const BUYER = process.env.BUYER ?? "demo.blindbook-dev.testnet";
const CRANKER = process.env.CRANKER ?? "blindbook-crank-e4c061.testnet";
const LOTS = Number(process.env.LOTS ?? 2);
const BAND_BPS = 50n;

const key = (id: string) =>
  JSON.parse(readFileSync(join(homedir(), ".near-credentials", "testnet", `${id}.json`), "utf8")).private_key as string;
const client = (id: string) => new Blindbook({ network: TESTNET, accountId: id, secretKey: key(id) });
const log = (...a: unknown[]) => console.log(...a);

const market = TESTNET.markets[0];
const seller = client(SELLER);
const buyer = client(BUYER);
const cranker = client(CRANKER);

async function balances() {
  const out: Record<string, { base: string; quote: string }> = {};
  for (const c of [seller, buyer]) {
    out[c.accountId] = { base: (await c.balance(market.base)).toString(), quote: (await c.balance(market.quote)).toString() };
  }
  return out;
}

// 1. reference midpoint from mainnet 1Click (the test tokens track wNEAR / USDC)
const mid = await referenceMid(MAINNET.markets[0], 10n * 10n ** 24n, 36_000_000n);
const lotMid = (market.lot * mid.quote) / mid.base;
const sellLimit = (lotMid * (10_000n - BAND_BPS)) / 10_000n;
const buyLimit = (lotMid * (10_000n + BAND_BPS)) / 10_000n;
log(`reference mid ≈ ${((mid.bid + mid.ask) / 2).toFixed(4)} · sell limit ${priceOfLot(market, sellLimit)} · buy limit ${priceOfLot(market, buyLimit)}`);

const before = await balances();

// 2. both sides seal and submit
const sell = await seller.placeOrder({ market: market.id, side: "sell", limits: Array(LOTS).fill(sellLimit), ttlMinutes: 30 });
log(`sell order ${sell.orderId} · ${sell.tx.hash} · ${sell.tx.ok ? "ok" : sell.tx.error}`);
const buy = await buyer.placeOrder({ market: market.id, side: "buy", limits: Array(LOTS).fill(buyLimit), ttlMinutes: 30 });
log(`buy order ${buy.orderId} · ${buy.tx.hash} · ${buy.tx.ok ? "ok" : buy.tx.error}`);
if (!sell.tx.ok || !buy.tx.ok) process.exit(1);

// 3. a third account cranks one batch
const cfg = await cranker.poolConfig();
const batchId = cfg.next_batch_id;
const crank = await cranker.runBatch();
log(`run_batch by ${CRANKER} · ${crank.hash} · ${crank.ok ? "ok" : crank.error}`);

// 4. read the batch record and balances
let batch: any = null;
for (let i = 0; i < 20; i++) {
  batch = await cranker.view(TESTNET.pool, "get_batch", { id: String(batchId) }).catch(() => null);
  if (batch && ["settled", "partially_settled", "failed", "rejected_signature", "no_cross"].includes(batch.status)) break;
  await new Promise((r) => setTimeout(r, 3_000));
}
const after = await balances();
const lotsFilled = await seller.lotStatus(sell.drafts);

const delta = (id: string, k: "base" | "quote") => BigInt(after[id][k]) - BigInt(before[id][k]);
const sellerQuote = delta(SELLER, "quote");
const buyerQuote = -delta(BUYER, "quote");
const filled = BigInt(lotsFilled.filter(Boolean).length);
const summary = {
  at: new Date().toISOString(),
  network: "testnet",
  pool: TESTNET.pool,
  verifier: TESTNET.verifier,
  reference_mid: (mid.bid + mid.ask) / 2,
  lots: LOTS,
  lots_filled: Number(filled),
  txs: { sell: sell.tx.hash, buy: buy.tx.hash, run_batch: crank.hash },
  batch,
  seller: { account: SELLER, base_delta: delta(SELLER, "base").toString(), quote_delta: sellerQuote.toString() },
  buyer: { account: BUYER, base_delta: delta(BUYER, "base").toString(), quote_delta: (-buyerQuote).toString() },
  checks: {
    seller_got_at_least_limit: filled > 0n && sellerQuote >= sellLimit * filled,
    buyer_paid_at_most_limit: filled > 0n && buyerQuote <= buyLimit * filled,
    seller_effective_price: filled > 0n ? priceOfLot(market, sellerQuote / filled) : null,
    buyer_effective_price: filled > 0n ? priceOfLot(market, buyerQuote / filled) : null,
  },
};
log(JSON.stringify(summary.checks, null, 2));
mkdirSync("results", { recursive: true });
const file = `results/e2e-testnet-${summary.at.replace(/[:.]/g, "-")}.json`;
writeFileSync(file, `${JSON.stringify(summary, null, 2)}\n`);
log(`wrote ${file}`);
