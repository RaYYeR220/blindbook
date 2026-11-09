import "server-only";
import { Blindbook } from "@blindbook/client";
import { MAINNET, type MultiPayload, referenceMid } from "@blindbook/sdk";
import { network } from "./env";

/**
 * Gas relay. Traders only sign: their lots are signed and sealed in the
 * browser, so the server can submit them without being able to read or alter
 * them. The relayer pays the storage deposit, which the pool refunds to it
 * once the matcher ingests the order.
 */
let relayer: Blindbook | null = null;
function relay(): Blindbook | null {
  if (relayer) return relayer;
  const accountId = process.env.RELAY_ACCOUNT_ID ?? process.env.DEMO_ACCOUNT_ID;
  const secretKey = process.env.RELAY_SECRET_KEY ?? process.env.DEMO_SECRET_KEY;
  if (!accountId || !secretKey) return null;
  relayer = new Blindbook({ network, accountId, secretKey });
  return relayer;
}

export const relayAvailable = () => relay() !== null;

const ENVELOPE_SIZES = new Set([2_048, 4_096, 8_192, 16_384, 32_768, 65_536].map((b) => 57 + b + 16));

export async function relaySubmit(market: string, sealedB64: string) {
  const r = relay();
  if (!r) throw new Error("relay is not configured");
  if (!network.markets.some((m) => m.id === market)) throw new Error("unknown market");
  const bytes = Buffer.from(sealedB64, "base64");
  if (!ENVELOPE_SIZES.has(bytes.length)) throw new Error("not a sealed order envelope");
  const deposit = BigInt(await r.view<string>(network.pool, "get_order_deposit", { bytes: bytes.length }));
  const tx = await r.call(network.pool, "submit_order", { market, sealed_b64: sealedB64 }, 50, deposit);
  return { ok: tx.ok, orderId: tx.ok && tx.value != null ? String(tx.value) : null, tx: tx.hash, error: tx.error };
}

/** Execute signed cancel tickets (empty-intent payloads) on the verifier. */
export async function relayCancel(tickets: MultiPayload[]) {
  const r = relay();
  if (!r) throw new Error("relay is not configured");
  if (tickets.length === 0 || tickets.length > 32) throw new Error("between 1 and 32 tickets");
  for (const t of tickets) {
    const message = t.standard === "nep413" ? t.payload.message : t.payload;
    const inner = JSON.parse(message) as { intents?: unknown[] };
    if (!Array.isArray(inner.intents) || inner.intents.length !== 0) throw new Error("only cancel tickets can be relayed");
  }
  const tx = await r.call(network.verifier, "execute_intents", { signed: tickets }, 150);
  return { ok: tx.ok, tx: tx.hash, error: tx.error };
}

/**
 * House liquidity: before a crank, the house rests a few lots on both sides a
 * band away from the midpoint, valid for ten minutes. The matcher never lets
 * it trade with itself and keeps only the side incoming flow needs, so unused
 * house lots simply expire.
 */
const HOUSE_LOTS = 3;
const HOUSE_BAND_BPS = 60n;
async function refreshHouse() {
  const id = process.env.HOUSE_ACCOUNT_ID;
  const key = process.env.HOUSE_SECRET_KEY;
  if (!id || !key) return;
  const house = new Blindbook({ network, accountId: id, secretKey: key });
  const market = network.markets[0];
  const mid = await referenceMid(MAINNET.markets[0], 10n * 10n ** 24n, 36_000_000n);
  const lotMid = (market.lot * mid.quote) / mid.base;
  const quotes = [
    { side: "buy" as const, limit: (lotMid * (10_000n + HOUSE_BAND_BPS)) / 10_000n },
    { side: "sell" as const, limit: (lotMid * (10_000n - HOUSE_BAND_BPS)) / 10_000n },
  ];
  for (const q of quotes) {
    await house.placeOrder({ market: market.id, side: q.side, limits: Array(HOUSE_LOTS).fill(q.limit), ttlMinutes: 10 }).catch(() => null);
  }
}

let lastCrank = 0;
export async function relayCrank() {
  const r = relay();
  if (!r) throw new Error("relay is not configured");
  if (Date.now() - lastCrank < 15_000) return { ok: false, error: "a batch was cranked moments ago" };
  const pending = (await r.poolConfig()).pending;
  if (pending === 0) return { ok: false, error: "nothing pending" };
  lastCrank = Date.now();
  await refreshHouse();
  const cfg = await r.poolConfig();
  const batchId = cfg.next_batch_id;
  const tx = await r.runBatch();
  return { ok: tx.ok, tx: tx.hash, batchId, cranker: r.accountId, error: tx.error };
}
