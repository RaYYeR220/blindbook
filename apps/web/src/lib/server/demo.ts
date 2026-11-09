import "server-only";
import { Blindbook } from "@blindbook/client";
import { MAINNET, priceOfLot, referenceMid } from "@blindbook/sdk";
import { demoKeys, explorer, network } from "./env";
import { type BatchRecord, forget, poolView } from "./pool";

/**
 * The public "try it" flow. One click:
 *   1. the house rests one lot on the opposite side, a band through the mid,
 *   2. the demo account seals one lot on the side the visitor picked,
 *   3. the demo account cranks a batch,
 *   4. the enclave crosses both at the verified midpoint and the pool settles them.
 *
 * Both accounts are server-held and only ever trade one lot per click.
 */

const BAND_BPS = 60n;
const COOLDOWN_MS = 20_000;
let lastRun = 0;
let running = false;

export type DemoSide = "sell" | "buy";

export interface DemoResult {
  ok: boolean;
  side: DemoSide;
  reason?: string;
  mid?: number;
  limit?: string;
  lot?: string;
  txs?: { house: string; order: string; crank: string };
  explorer?: string;
  batch?: BatchRecord | null;
  effective?: string | null;
}

function clients() {
  const keys = demoKeys();
  if (!keys) return null;
  return {
    demo: new Blindbook({ network, accountId: keys.demo.accountId, secretKey: keys.demo.secretKey }),
    house: new Blindbook({ network, accountId: keys.house.accountId, secretKey: keys.house.secretKey }),
  };
}

export function demoAvailable() {
  return demoKeys() !== null;
}

export async function runDemo(requested: DemoSide): Promise<DemoResult> {
  const c = clients();
  if (!c) return { ok: false, side: requested, reason: "demo is not configured on this deployment" };
  if (running || Date.now() - lastRun < COOLDOWN_MS) {
    return { ok: false, side: requested, reason: "a demo batch just ran; try again in a few seconds" };
  }
  running = true;
  try {
    const market = network.markets[0];
    // Orders other people have pending simply join the same batch.

    // pick a side the demo account can actually fund, preferring the visitor's choice
    const mid = await referenceMid(MAINNET.markets[0], 10n * 10n ** 24n, 36_000_000n);
    const lotMid = (market.lot * mid.quote) / mid.base;
    const sellLimit = (lotMid * (10_000n - BAND_BPS)) / 10_000n;
    const buyLimit = (lotMid * (10_000n + BAND_BPS)) / 10_000n;
    const [baseBal, quoteBal] = await Promise.all([c.demo.balance(market.base), c.demo.balance(market.quote)]);
    const canSell = baseBal >= market.lot;
    const canBuy = quoteBal >= buyLimit;
    let side: DemoSide = requested;
    if (side === "sell" && !canSell && canBuy) side = "buy";
    if (side === "buy" && !canBuy && canSell) side = "sell";
    if ((side === "sell" && !canSell) || (side === "buy" && !canBuy)) {
      return { ok: false, side: requested, reason: "the demo account is out of inventory; try the other side" };
    }

    // the house takes the other side of exactly one lot
    const houseSide: DemoSide = side === "sell" ? "buy" : "sell";
    const house = await c.house.placeOrder({
      market: market.id,
      side: houseSide,
      limits: [houseSide === "sell" ? sellLimit : buyLimit],
      ttlMinutes: 10,
    });
    if (!house.tx.ok) return { ok: false, side, reason: `house order failed: ${house.tx.error?.slice(0, 160)}` };

    const limit = side === "sell" ? sellLimit : buyLimit;
    const order = await c.demo.placeOrder({ market: market.id, side, limits: [limit], ttlMinutes: 10 });
    if (!order.tx.ok) return { ok: false, side, reason: `order failed: ${order.tx.error?.slice(0, 160)}` };

    const batchId = (await c.demo.poolConfig()).next_batch_id;
    const crank = await c.demo.runBatch();
    lastRun = Date.now();
    forget();

    const batch = await poolView<BatchRecord | null>("get_batch", { id: String(batchId) }, 0).catch(() => null);
    const m = batch?.markets?.find((x) => x.market === market.id);
    // what one lot actually settled at: mid +/- the venue fee share for this side
    let effective: string | null = null;
    if (m && m.matched_lots > 0) {
      const lotMidQuote = BigInt(m.lot_mid_quote);
      const feePerLot = BigInt(m.venue_fee) / BigInt(2 * m.matched_lots);
      effective = priceOfLot(market, side === "sell" ? lotMidQuote - feePerLot : lotMidQuote + feePerLot);
    }
    return {
      ok: crank.ok && batch?.status === "settled",
      side,
      reason: crank.ok ? undefined : crank.error?.slice(0, 200),
      mid: (mid.bid + mid.ask) / 2,
      limit: priceOfLot(market, limit),
      lot: `${priceOfLot({ ...market, quoteDecimals: market.baseDecimals, lot: 10n ** BigInt(market.baseDecimals) }, market.lot)} ${market.baseSymbol}`,
      txs: { house: house.tx.hash, order: order.tx.hash, crank: crank.hash },
      explorer,
      batch,
      effective,
    };
  } finally {
    running = false;
  }
}
