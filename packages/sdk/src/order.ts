/**
 * Building Blindbook orders: a bundle of pre-signed lots, each one
 * `token_diff` for exactly one lot at the trader's own limit, sealed to the
 * matcher's enclave key.
 */
import { base64, hex } from "@scure/base";
import { sealOrder } from "./envelope.js";
import {
  buildPayload,
  cancelTicket,
  type IntentPayload,
  makeNonce,
  type MultiPayload,
  tokenDiff,
} from "./intents.js";

export type Side = "sell" | "buy";

export interface Market {
  /** Market id used by the pool, e.g. `wnear-usdc`. */
  id: string;
  /** Verifier asset ids. */
  base: string;
  quote: string;
  /** Base units per lot. */
  lot: bigint;
  baseDecimals: number;
  quoteDecimals: number;
  baseSymbol: string;
  quoteSymbol: string;
}

/** Price as a decimal string in quote per one whole base unit, e.g. "3.54". */
export function limitForLot(market: Market, price: string): bigint {
  const [whole, frac = ""] = price.trim().split(".");
  if (!/^\d+$/.test(whole) || !/^\d*$/.test(frac)) throw new Error(`invalid price ${price}`);
  // quote units per lot = price * 10^quoteDecimals * lot / 10^baseDecimals
  const scale = 10n ** BigInt(frac.length);
  const priceScaled = BigInt(whole) * scale + BigInt(frac || "0");
  return (priceScaled * 10n ** BigInt(market.quoteDecimals) * market.lot) / (scale * 10n ** BigInt(market.baseDecimals));
}

/** Price per whole base unit implied by a lot limit, for display. */
export function priceOfLot(market: Market, limitQuote: bigint, digits = 6): string {
  const num = limitQuote * 10n ** BigInt(market.baseDecimals) * 10n ** BigInt(digits);
  const den = market.lot * 10n ** BigInt(market.quoteDecimals);
  const v = num / den;
  const s = v.toString().padStart(digits + 1, "0");
  return `${s.slice(0, -digits)}.${s.slice(-digits)}`.replace(/\.?0+$/, "");
}

/** The diff one lot commits to. */
export function lotDiff(market: Market, side: Side, limitQuote: bigint): Record<string, bigint> {
  return side === "sell"
    ? { [market.base]: -market.lot, [market.quote]: limitQuote }
    : { [market.base]: market.lot, [market.quote]: -limitQuote };
}

export interface LotDraft {
  index: number;
  limitQuote: bigint;
  payload: IntentPayload;
}

/**
 * Unsigned lot payloads for an order. `limits` holds one quote limit per lot,
 * which lets a trader ladder their price.
 */
export function draftLots(opts: {
  market: Market;
  side: Side;
  signerId: string;
  verifier: string;
  salt: string;
  limits: bigint[];
  /** When the lots stop being valid. */
  deadline: Date;
}): LotDraft[] {
  if (opts.limits.length === 0) throw new Error("an order needs at least one lot");
  return opts.limits.map((limitQuote, index) => ({
    index,
    limitQuote,
    payload: buildPayload({
      signerId: opts.signerId,
      verifier: opts.verifier,
      deadline: opts.deadline,
      nonce: makeNonce(opts.salt, opts.deadline),
      intents: [tokenDiff(lotDiff(opts.market, opts.side, limitQuote))],
    }),
  }));
}

export interface OrderBody {
  v: 1;
  market: string;
  owner: string;
  side: Side;
  min_lots: number;
  lots: { index: number; limit_quote: string; intent: MultiPayload }[];
}

export function orderBody(opts: {
  market: Market;
  owner: string;
  side: Side;
  minLots?: number;
  lots: { draft: LotDraft; signed: MultiPayload }[];
}): OrderBody {
  return {
    v: 1,
    market: opts.market.id,
    owner: opts.owner,
    side: opts.side,
    min_lots: opts.minLots ?? 1,
    lots: opts.lots.map(({ draft, signed }) => ({
      index: draft.index,
      limit_quote: draft.limitQuote.toString(),
      intent: signed,
    })),
  };
}

/** Seal an order body to the matcher key (hex) and return base64 for `submit_order`. */
export function sealOrderBody(matcherKeyHex: string, body: OrderBody): string {
  return base64.encode(sealOrder(hex.decode(matcherKeyHex), JSON.stringify(body)));
}

/**
 * Cancel tickets for every lot of an order. Signing and submitting these to
 * the verifier consumes the lots' nonces, so they can never be filled. No
 * permission from the venue is needed.
 */
export function cancelTickets(drafts: LotDraft[]): IntentPayload[] {
  return drafts.map((d) => cancelTicket(d.payload));
}
