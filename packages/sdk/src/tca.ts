/**
 * Transaction-cost analysis against the public route: what the same block
 * would cost as a 1Click swap right now. Dry quotes are free and need no key.
 */
import type { Market, Side } from "./order.js";

export const ONE_CLICK = "https://1click.chaindefuser.com/v0";

export interface RouteQuote {
  ok: true;
  amountIn: bigint;
  amountOut: bigint;
  signature: string;
  timestamp: string;
}
export interface RouteFailure {
  ok: false;
  status: number;
  message: string;
}

/** Ask 1Click for a dry EXACT_INPUT quote between two verifier assets. */
export async function dryQuote(origin: string, destination: string, amountIn: bigint, account = "blindbook.near"): Promise<RouteQuote | RouteFailure> {
  const body = {
    dry: true,
    swapType: "EXACT_INPUT",
    slippageTolerance: 100,
    originAsset: origin,
    depositType: "INTENTS",
    destinationAsset: destination,
    amount: amountIn.toString(),
    refundTo: account,
    refundType: "INTENTS",
    recipient: account,
    recipientType: "INTENTS",
    deadline: new Date(Date.now() + 30 * 60_000).toISOString(),
  };
  const res = await fetch(`${ONE_CLICK}/quote`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const j = (await res.json()) as {
    message?: string;
    signature?: string;
    timestamp?: string;
    quote?: { amountIn: string; amountOut: string };
  };
  if (!res.ok || !j.quote) return { ok: false, status: res.status, message: j.message ?? `HTTP ${res.status}` };
  return {
    ok: true,
    amountIn: BigInt(j.quote.amountIn),
    amountOut: BigInt(j.quote.amountOut),
    signature: j.signature ?? "",
    timestamp: j.timestamp ?? "",
  };
}

export interface RouteCost {
  side: Side;
  baseAmount: bigint;
  /** Quote received (sell) or paid (buy) on the public route; null when no route exists. */
  routeQuote: bigint | null;
  /** Quote the same size is worth at the reference midpoint. */
  midQuote: bigint;
  /** Cost of the public route versus the midpoint, in basis points. */
  costBps: number | null;
  failure?: string;
}

/**
 * Compare the public route for `baseAmount` with the midpoint. `midQuotePerBase`
 * is the midpoint expressed as quote units per `midBase` base units.
 */
export async function routeCost(
  market: Market,
  side: Side,
  baseAmount: bigint,
  mid: { quote: bigint; base: bigint },
): Promise<RouteCost> {
  const midQuote = (baseAmount * mid.quote) / mid.base;
  if (side === "sell") {
    const q = await dryQuote(market.base, market.quote, baseAmount);
    if (!q.ok) return { side, baseAmount, routeQuote: null, midQuote, costBps: null, failure: q.message };
    const cost = Number(((midQuote - q.amountOut) * 1_000_000n) / midQuote) / 100;
    return { side, baseAmount, routeQuote: q.amountOut, midQuote, costBps: cost };
  }
  // buying base: spend the mid value of quote and see how much base comes back
  const q = await dryQuote(market.quote, market.base, midQuote);
  if (!q.ok) return { side, baseAmount, routeQuote: null, midQuote, costBps: null, failure: q.message };
  const shortfall = baseAmount - q.amountOut;
  const cost = Number((shortfall * 1_000_000n) / baseAmount) / 100;
  return { side, baseAmount, routeQuote: midQuote, midQuote, costBps: cost };
}

/** Midpoint from two small dry quotes (a bid and an ask), as quote per base. */
export async function referenceMid(market: Market, probeBase: bigint, probeQuote: bigint): Promise<{ quote: bigint; base: bigint; bid: number; ask: number }> {
  const bid = await dryQuote(market.base, market.quote, probeBase);
  const ask = await dryQuote(market.quote, market.base, probeQuote);
  if (!bid.ok || !ask.ok) throw new Error("reference quotes unavailable");
  // bid = bidOut/probeBase, ask = probeQuote/askOut; mid on a common base
  const base = probeBase * ask.amountOut;
  const quote = (bid.amountOut * ask.amountOut + probeQuote * probeBase) / 2n;
  const unit = 10n ** BigInt(market.baseDecimals);
  const toFloat = (q: bigint, b: bigint) => Number((q * unit * 1_000_000n) / b) / 10 ** (market.quoteDecimals + 6);
  return { quote, base, bid: toFloat(bid.amountOut, probeBase), ask: toFloat(probeQuote, ask.amountOut) };
}
