import {
  cancelTickets,
  draftLots,
  type IntentPayload,
  type LotDraft,
  limitForLot,
  type Market,
  type MultiPayload,
  orderBody,
  sealOrderBody,
  type Side,
} from "@blindbook/sdk";
import { readJson, writeJson } from "./storage";
import type { BatchMarketView, BatchView, RelaySubmitResponse } from "./types";
import type { Signed, WalletKind } from "./wallet";

/**
 * Orders as the browser remembers them: the unsigned lot payloads (public
 * once settled anyway), their nonces, and the transactions that touched
 * them. Signatures and keys are never written down.
 */

export interface StoredLot {
  index: number;
  limitQuote: string;
  payload: IntentPayload;
}

export interface StoredOrder {
  key: string;
  account: string;
  kind: WalletKind;
  market: string;
  side: Side;
  minLots: number;
  createdAt: number;
  deadline: string;
  orderId: string | null;
  submitTx: string;
  /** The pool's next batch id when the order was submitted: no earlier batch can hold it. */
  batchFloor: number;
  envelopeBytes: number;
  lots: StoredLot[];
  cancel?: { tx: string; at: number; nonces: string[] };
  /** nonce → the batch the lot crossed in, once seen. */
  fills?: Record<string, string>;
}

const ordersKey = (network: string, account: string) => `bb:orders:v1:${network}:${account}`;

export const loadOrders = (network: string, account: string) => readJson<StoredOrder[]>(ordersKey(network, account), []);

export const saveOrders = (network: string, account: string, orders: StoredOrder[]) =>
  writeJson(ordersKey(network, account), orders.slice(0, 40));

// ---------------------------------------------------------------- planning

/**
 * One quote limit per lot. A ladder steps each next lot's limit away from the
 * first by `ladderBps`, in the trader's favour: up for a sell, down for a buy,
 * so no lot is ever priced worse than the first.
 */
export function ladderLimits(market: Market, side: Side, price: string, lots: number, ladderBps: number): bigint[] {
  const first = limitForLot(market, price);
  const step = BigInt(Math.max(0, Math.round(ladderBps)));
  return Array.from({ length: lots }, (_, i) => {
    const k = BigInt(i) * step;
    return side === "sell" ? (first * (10_000n + k)) / 10_000n : (first * (10_000n - k)) / 10_000n;
  });
}

/** Bytes in a base64 string, without decoding it. */
export const base64Bytes = (b64: string) =>
  Math.floor((b64.length * 3) / 4) - (b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0);

// ---------------------------------------------------------------- sealing

export type SealEvent =
  | { step: "sign"; done: number; total: number }
  | { step: "key"; state: "checking" | "registering" | "ok"; tx?: string }
  | { step: "encrypt"; bytes: number; matcherKey: string }
  | { step: "submit"; state: "sending" | "ok"; tx?: string; orderId?: string | null };

export interface SealInput {
  market: Market;
  verifier: string;
  account: string;
  kind: WalletKind;
  side: Side;
  limits: bigint[];
  minLots: number;
  ttlMinutes: number;
  /** The verifier's current salt, for the versioned nonces. */
  salt: string;
  /** The matcher's X25519 key, hex, as `pool.get_matcher_key()` publishes it. */
  matcherKey: string;
  /** The pool's next batch id right now. */
  nextBatchId: number;
  sign: (payload: IntentPayload) => Promise<Signed>;
  /**
   * Called once with the key that signed the first lot, before anything is
   * submitted, so a named NEAR account can register it with the verifier.
   */
  ensureKey?: (publicKey: string) => Promise<void>;
  submit: (market: string, sealedB64: string) => Promise<RelaySubmitResponse>;
  onEvent?: (e: SealEvent) => void;
  /** Lets a test pin the clock. */
  now?: () => number;
}

/**
 * Draft one payload per lot, have the wallet sign each, seal the bundle to
 * the enclave key, and hand the ciphertext to the relay. Every lot's signed
 * diff is its own price floor: the verifier applies it exactly or not at all.
 */
export async function sealAndSubmit(input: SealInput): Promise<StoredOrder> {
  const now = input.now ?? Date.now;
  const emit = input.onEvent ?? (() => {});
  const createdAt = now();
  const deadline = new Date(createdAt + input.ttlMinutes * 60_000);
  const drafts = draftLots({
    market: input.market,
    side: input.side,
    signerId: input.account,
    verifier: input.verifier,
    salt: input.salt,
    limits: input.limits,
    deadline,
  });

  const signed: { draft: LotDraft; signed: MultiPayload }[] = [];
  emit({ step: "sign", done: 0, total: drafts.length });
  for (const draft of drafts) {
    const s = await input.sign(draft.payload);
    if (signed.length === 0 && s.publicKey && input.ensureKey) {
      emit({ step: "key", state: "checking" });
      await input.ensureKey(s.publicKey);
      emit({ step: "key", state: "ok" });
    }
    signed.push({ draft, signed: s.signed });
    emit({ step: "sign", done: signed.length, total: drafts.length });
  }

  const body = orderBody({ market: input.market, owner: input.account, side: input.side, minLots: input.minLots, lots: signed });
  const sealed = sealOrderBody(input.matcherKey, body);
  const bytes = base64Bytes(sealed);
  emit({ step: "encrypt", bytes, matcherKey: input.matcherKey });

  emit({ step: "submit", state: "sending" });
  const res = await input.submit(input.market.id, sealed);
  if (!res.ok) throw new Error(res.error ? `the pool refused the order: ${res.error.slice(0, 200)}` : "the relay could not submit the order");
  emit({ step: "submit", state: "ok", tx: res.tx, orderId: res.orderId });

  return {
    key: `${createdAt.toString(36)}-${res.tx.slice(0, 8)}`,
    account: input.account,
    kind: input.kind,
    market: input.market.id,
    side: input.side,
    minLots: input.minLots,
    createdAt,
    deadline: deadline.toISOString(),
    orderId: res.orderId,
    submitTx: res.tx,
    batchFloor: input.nextBatchId,
    envelopeBytes: bytes,
    lots: drafts.map((d) => ({ index: d.index, limitQuote: d.limitQuote.toString(), payload: d.payload })),
  };
}

/** Cancel tickets for the given lots: the same nonce, no intents. Signed one by one. */
export async function signCancelTickets(
  lots: StoredLot[],
  sign: (payload: IntentPayload) => Promise<Signed>,
  onSigned?: (done: number, total: number) => void,
): Promise<MultiPayload[]> {
  const drafts: LotDraft[] = lots.map((l) => ({ index: l.index, limitQuote: BigInt(l.limitQuote), payload: l.payload }));
  const out: MultiPayload[] = [];
  for (const ticket of cancelTickets(drafts)) {
    out.push((await sign(ticket)).signed);
    onSigned?.(out.length, drafts.length);
  }
  return out;
}

// ---------------------------------------------------------------- status

export type LotState = "pending" | "resting" | "crossed" | "cancelled" | "expired" | "refused";

export interface LotStatus {
  lot: StoredLot;
  state: LotState;
  /** the batch a crossed lot settled in */
  batch?: BatchView;
  /** why the matcher refused the order */
  reason?: string;
}

/** What one lot of a batch settled at for the given side: the mid, less (sell) or plus (buy) its share of the venue fee. */
export function fillQuote(m: BatchMarketView, side: Side): bigint | null {
  if (m.matched_lots <= 0) return null;
  const mid = BigInt(m.lot_mid_quote);
  const fee = BigInt(m.venue_fee) / BigInt(2 * m.matched_lots);
  return side === "sell" ? mid - fee : mid + fee;
}

/** The earliest settled batch at or after `floor` in which this market crossed lots. */
export function crossingBatch(batches: BatchView[], market: string, floor: number): BatchView | undefined {
  return [...batches]
    .filter((b) => Number(b.id) >= floor && b.status === "settled")
    .sort((a, b) => Number(a.id) - Number(b.id))
    .find((b) => b.markets.some((m) => m.market === market && m.matched_lots > 0));
}

/**
 * A lot's state from public facts only: whether its nonce is used on the
 * verifier, whether we sent a cancel ticket for it, the order's deadline, and
 * the batches' post-trade records.
 */
export function lotStatuses(order: StoredOrder, used: Record<string, boolean>, batches: BatchView[], now: number): LotStatus[] {
  const ingestedIn = order.orderId ? batches.find((b) => b.ingested.includes(order.orderId!)) : undefined;
  const refusal = order.orderId
    ? batches.flatMap((b) => b.rejected).find((r) => r.id === order.orderId)
    : undefined;
  const floor = ingestedIn ? Number(ingestedIn.id) : order.batchFloor;
  const expired = Date.parse(order.deadline) <= now;
  return order.lots.map((lot) => {
    const nonce = lot.payload.nonce;
    // a lot cancelled before ingestion comes back rejected, since its nonce is spent: it is still a cancel
    if (order.cancel?.nonces.includes(nonce)) return { lot, state: "cancelled" };
    if (refusal) return { lot, state: "refused", reason: refusal.reason };
    if (used[nonce] || order.fills?.[nonce]) {
      const id = order.fills?.[nonce];
      const batch = id ? batches.find((b) => b.id === id) : crossingBatch(batches, order.market, floor);
      return { lot, state: "crossed", batch };
    }
    if (expired) return { lot, state: "expired" };
    return { lot, state: ingestedIn ? "resting" : "pending" };
  });
}
