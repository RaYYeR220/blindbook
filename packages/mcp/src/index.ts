#!/usr/bin/env node
/**
 * Blindbook MCP server.
 *
 * Read-only tools work with no configuration. Trading tools need the agent's
 * own NEAR key:
 *
 *   BLINDBOOK_NETWORK=mainnet|testnet   (default testnet)
 *   BLINDBOOK_ACCOUNT_ID=agent.near     (optional; implicit account of the key otherwise)
 *   BLINDBOOK_SECRET_KEY=ed25519:...
 *   BLINDBOOK_MAX_LOTS=10               (hard cap per order, default 10)
 *
 * The key never leaves this process. Orders are sealed locally to the
 * matcher's enclave key before they are submitted.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { Blindbook, type LotDraft } from "@blindbook/client";
import { limitForLot, MAINNET, type Network, priceOfLot, referenceMid, routeCost, rpcView, TESTNET } from "@blindbook/sdk";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const network: Network = process.env.BLINDBOOK_NETWORK === "mainnet" ? MAINNET : TESTNET;
const maxLots = Number(process.env.BLINDBOOK_MAX_LOTS ?? 10);

let client: Blindbook | null = null;
function trader(): Blindbook {
  if (client) return client;
  const secretKey = process.env.BLINDBOOK_SECRET_KEY;
  if (!secretKey) throw new Error("set BLINDBOOK_SECRET_KEY to trade; read-only tools work without it");
  client = new Blindbook({ network, secretKey, accountId: process.env.BLINDBOOK_ACCOUNT_ID });
  return client;
}

// Lot drafts (payloads and nonces, never keys) are kept locally so orders can
// be tracked and cancelled across calls.
const storeDir = join(homedir(), ".blindbook");
const storeFile = () => join(storeDir, `orders-${network.networkId}-${trader().accountId}.json`);
type Stored = { orderId: string | null; market: string; side: string; placedAt: string; drafts: LotDraft[] };
function loadOrders(): Record<string, Stored> {
  try {
    return JSON.parse(readFileSync(storeFile(), "utf8"), (_k, v) => (typeof v === "string" && /^-?\d+n$/.test(v) ? BigInt(v.slice(0, -1)) : v));
  } catch {
    return {};
  }
}
function saveOrders(orders: Record<string, Stored>) {
  mkdirSync(storeDir, { recursive: true });
  writeFileSync(storeFile(), JSON.stringify(orders, (_k, v) => (typeof v === "bigint" ? `${v}n` : v), 2));
}

const text = (v: unknown) => ({ content: [{ type: "text" as const, text: typeof v === "string" ? v : JSON.stringify(v, null, 2) }] });

const server = new McpServer({ name: "blindbook", version: "0.1.0" });

server.registerTool(
  "blindbook_markets",
  {
    title: "List Blindbook markets",
    description: "Markets, lot sizes and the contracts Blindbook settles through on the configured network.",
    inputSchema: {},
    annotations: { readOnlyHint: true },
  },
  async () =>
    text({
      network: network.networkId,
      pool: network.pool,
      verifier: network.verifier,
      markets: network.markets.map((m) => ({
        id: m.id,
        base: `${m.baseSymbol} (${m.base})`,
        quote: `${m.quoteSymbol} (${m.quote})`,
        lot: `${priceOfLot({ ...m, quoteDecimals: m.baseDecimals, lot: 10n ** BigInt(m.baseDecimals) }, m.lot)} ${m.baseSymbol}`,
      })),
    }),
);

server.registerTool(
  "blindbook_route_cost",
  {
    title: "Price a block on the public route",
    description:
      "What selling or buying a block of NEAR would cost right now as a public NEAR Intents swap, versus the midpoint Blindbook crosses at. Uses free 1Click dry quotes on mainnet.",
    inputSchema: {
      side: z.enum(["sell", "buy"]),
      near: z.number().positive().describe("Block size in NEAR"),
    },
    annotations: { readOnlyHint: true, openWorldHint: true },
  },
  async ({ side, near }) => {
    const market = MAINNET.markets[0];
    const base = BigInt(Math.round(near * 1e6)) * 10n ** 18n;
    const mid = await referenceMid(market, 10n * 10n ** 24n, 36_000_000n);
    const r = await routeCost(market, side, base, mid);
    return text({
      side,
      near,
      mid_usdc_per_near: (mid.bid + mid.ask) / 2,
      value_at_mid_usdc: Number(r.midQuote) / 1e6,
      public_route_cost_bps: r.costBps,
      public_route_cost_usdc: r.costBps === null ? null : (Number(r.midQuote) / 1e6) * (r.costBps / 10_000),
      public_route: r.costBps === null ? `unavailable: ${r.failure}` : "available",
      blindbook_cost_bps: "venue fee of up to 5 bps, taken only from price improvement",
    });
  },
);

server.registerTool(
  "blindbook_batches",
  {
    title: "Recent Blindbook batches",
    description: "The latest batches the pool ran: matched lots, volumes, the reference midpoint and settlement results.",
    inputSchema: { limit: z.number().int().min(1).max(50).default(10) },
    annotations: { readOnlyHint: true },
  },
  async ({ limit }) => {
    const stats = await rpcView(network.rpc, network.pool, "get_stats").catch((e) => `unavailable: ${e.message}`);
    const batches = await rpcView(network.rpc, network.pool, "get_batches", { from: null, limit }).catch(
      (e) => `unavailable: ${e.message}`,
    );
    return text({ stats, batches });
  },
);

server.registerTool(
  "blindbook_balance",
  {
    title: "Verifier balances",
    description: "This agent's balances inside the NEAR Intents verifier for every Blindbook market token.",
    inputSchema: {},
    annotations: { readOnlyHint: true },
  },
  async () => {
    const bb = trader();
    const tokens = [...new Set(network.markets.flatMap((m) => [m.base, m.quote]))];
    const out: Record<string, string> = {};
    for (const t of tokens) out[t] = (await bb.balance(t)).toString();
    return text({ account: bb.accountId, balances: out });
  },
);

server.registerTool(
  "blindbook_place_order",
  {
    title: "Place a sealed block order",
    description:
      "Sell or buy a block in lots at your own limit price. Each lot is signed locally at the limit, sealed to the matcher's enclave key and submitted. It can only ever fill at the batch midpoint, and never worse than the limit. Unfilled lots expire at the deadline.",
    inputSchema: {
      market: z.string().default(network.markets[0].id),
      side: z.enum(["sell", "buy"]),
      lots: z.number().int().min(1),
      limit_price: z.string().describe("Limit in quote per whole base unit, e.g. \"3.40\""),
      min_lots: z.number().int().min(1).default(1),
      ttl_minutes: z.number().int().min(5).max(24 * 60).default(60),
    },
    annotations: { destructiveHint: false, openWorldHint: true },
  },
  async ({ market, side, lots, limit_price, min_lots, ttl_minutes }) => {
    if (lots > maxLots) throw new Error(`order of ${lots} lots exceeds BLINDBOOK_MAX_LOTS=${maxLots}`);
    const bb = trader();
    const m = bb.market(market);
    await bb.ensureVerifierKey();
    const limit = limitForLot(m, limit_price);
    const placed = await bb.placeOrder({ market, side, limits: Array(lots).fill(limit), minLots: min_lots, ttlMinutes: ttl_minutes });
    if (!placed.tx.ok) throw new Error(`submit_order failed: ${placed.tx.error}`);
    const key = placed.orderId ?? placed.tx.hash;
    const orders = loadOrders();
    orders[key] = { orderId: placed.orderId, market, side, placedAt: new Date().toISOString(), drafts: placed.drafts };
    saveOrders(orders);
    return text({ order: key, tx: placed.tx.hash, lots, limit_price, ttl_minutes, note: "Sealed. Only the enclave can read it." });
  },
);

server.registerTool(
  "blindbook_order_status",
  {
    title: "Order status",
    description: "How many lots of an order are still open versus consumed (filled or cancelled).",
    inputSchema: { order: z.string() },
    annotations: { readOnlyHint: true },
  },
  async ({ order }) => {
    const stored = loadOrders()[order];
    if (!stored) throw new Error(`unknown order ${order}`);
    const used = await trader().lotStatus(stored.drafts);
    return text({ order, lots: used.length, consumed: used.filter(Boolean).length, open: used.filter((u) => !u).length });
  },
);

server.registerTool(
  "blindbook_cancel_order",
  {
    title: "Cancel an order",
    description:
      "Cancel every open lot of an order by executing cancel tickets on the NEAR Intents verifier. Takes effect at once and needs no permission from the venue.",
    inputSchema: { order: z.string() },
    annotations: { destructiveHint: true },
  },
  async ({ order }) => {
    const orders = loadOrders();
    const stored = orders[order];
    if (!stored) throw new Error(`unknown order ${order}`);
    const tx = await trader().cancel(stored.drafts);
    return text(tx ? { cancelled: true, tx: tx.hash, ok: tx.ok } : { cancelled: false, reason: "no open lots" });
  },
);

await server.connect(new StdioServerTransport());
