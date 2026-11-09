/**
 * House liquidity and keeper.
 *
 * Rests a few lots on each side with limits a band away from the reference
 * midpoint, so any compatible order crosses against it at the exact midpoint.
 * The matcher never lets the house trade with itself: when it rests on both
 * sides, it only keeps the side incoming flow needs.
 *
 * It also cranks `run_batch` whenever the pool has pending orders, and on a
 * slow timer so resting orders get re-checked as the midpoint moves.
 *
 *   BLINDBOOK_NETWORK=testnet BLINDBOOK_ACCOUNT_ID=house.blindbook-dev.testnet \
 *   BLINDBOOK_SECRET_KEY=ed25519:... node dist/index.js
 */
import { Blindbook, type LotDraft } from "@blindbook/client";
import { MAINNET, type Market, referenceMid, rpcView, TESTNET } from "@blindbook/sdk";

const env = (k: string, d?: string) => {
  const v = process.env[k] ?? d;
  if (v === undefined) throw new Error(`missing ${k}`);
  return v;
};

const network = env("BLINDBOOK_NETWORK", "testnet") === "mainnet" ? MAINNET : TESTNET;
const bb = new Blindbook({ network, secretKey: env("BLINDBOOK_SECRET_KEY"), accountId: process.env.BLINDBOOK_ACCOUNT_ID });
const market: Market = bb.market(env("HOUSE_MARKET", network.markets[0].id));
const lotsPerSide = Number(env("HOUSE_LOTS", "2"));
const bandBps = BigInt(env("HOUSE_BAND_BPS", "50"));
const refreshMs = Number(env("HOUSE_REFRESH_MIN", "20")) * 60_000;
const crankEveryMs = Number(env("HOUSE_CRANK_MIN", "10")) * 60_000;
const pollMs = Number(env("HOUSE_POLL_S", "15")) * 1_000;
const requote = BigInt(env("HOUSE_REQUOTE_BPS", "40"));

// Reference prices come from mainnet assets; on testnet the test tokens track them.
const reference = MAINNET.markets[0];

interface Quote {
  placedAt: number;
  mid: { quote: bigint; base: bigint };
  orders: { side: "sell" | "buy"; drafts: LotDraft[] }[];
}
let current: Quote | null = null;
let lastCrank = 0;

const log = (...a: unknown[]) => console.log(new Date().toISOString(), ...a);

function lotLimit(mid: { quote: bigint; base: bigint }, side: "sell" | "buy"): bigint {
  // quote per lot at mid, in this market's quote units (test tokens share decimals with the reference)
  const atMid = (market.lot * mid.quote) / mid.base;
  return side === "sell" ? (atMid * (10_000n - bandBps)) / 10_000n : (atMid * (10_000n + bandBps)) / 10_000n;
}

function moved(a: { quote: bigint; base: bigint }, b: { quote: bigint; base: bigint }): bigint {
  const pa = (a.quote * 10n ** 12n) / a.base;
  const pb = (b.quote * 10n ** 12n) / b.base;
  const d = pa > pb ? pa - pb : pb - pa;
  return (d * 10_000n) / pb;
}

async function placeQuotes(mid: { quote: bigint; base: bigint }) {
  if (current) {
    for (const o of current.orders) {
      const tx = await bb.cancel(o.drafts);
      if (tx) log(`cancelled ${o.side} lots`, tx.hash, tx.ok ? "ok" : tx.error);
    }
  }
  const orders: Quote["orders"] = [];
  for (const side of ["buy", "sell"] as const) {
    const limit = lotLimit(mid, side);
    const placed = await bb.placeOrder({
      market: market.id,
      side,
      limits: Array(lotsPerSide).fill(limit),
      ttlMinutes: Math.ceil((refreshMs * 2) / 60_000),
    });
    log(`placed ${side} ${lotsPerSide} lots @ ${limit} per lot`, placed.tx.hash, placed.tx.ok ? `order ${placed.orderId}` : placed.tx.error);
    if (placed.tx.ok) orders.push({ side, drafts: placed.drafts });
  }
  current = { placedAt: Date.now(), mid, orders };
}

async function pendingCount(): Promise<number> {
  const p = await rpcView<unknown[]>(network.rpc, network.pool, "get_pending", { from: null, limit: 64 }).catch(() => []);
  return Array.isArray(p) ? p.length : 0;
}

async function tick() {
  const mid = await referenceMid(reference, 10n * 10n ** 24n, 36_000_000n);
  if (!current || Date.now() - current.placedAt > refreshMs || moved(mid, current.mid) > requote) {
    await placeQuotes(mid);
  }
  const pending = await pendingCount();
  if (pending > 0 || Date.now() - lastCrank > crankEveryMs) {
    const tx = await bb.runBatch();
    lastCrank = Date.now();
    log(`run_batch (pending ${pending})`, tx.hash, tx.ok ? "ok" : tx.error);
  }
}

log(`house on ${network.networkId} as ${bb.accountId}, ${lotsPerSide} lots/side, band ${bandBps} bps`);
await bb.ensureVerifierKey();
for (;;) {
  try {
    await tick();
  } catch (e) {
    log("tick failed:", (e as Error).message);
  }
  await new Promise((r) => setTimeout(r, pollMs));
}
