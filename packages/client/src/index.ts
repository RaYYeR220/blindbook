/**
 * Blindbook client for programs that hold their own NEAR key: agents,
 * treasuries, market makers and scripts.
 */
import {
  balanceOf,
  cancelTickets,
  draftLots,
  fetchSalt,
  implicitAccountOf,
  isNonceUsed,
  type LotDraft,
  type Market,
  type MultiPayload,
  type Network,
  orderBody,
  publicKeyOf,
  rpcView,
  sealOrderBody,
  seedFromNearKey,
  type Side,
  signNep413,
} from "@blindbook/sdk";
import { Account, actions, JsonRpcProvider, KeyPairSigner } from "near-api-js";

const TGAS = 10n ** 12n;
const ONE_YOCTO = 1n;

export interface TxOutcome {
  hash: string;
  ok: boolean;
  error?: string;
  value?: unknown;
  logs: string[];
}

function summarize(res: any): TxOutcome {
  const receipts: any[] = res.receipts_outcome ?? [];
  const logs = receipts.flatMap((r) => r.outcome.logs as string[]);
  const failures = receipts.filter((r) => JSON.stringify(r.outcome.status).includes("Failure"));
  const st = res.status ?? {};
  const ok = st.SuccessValue !== undefined && failures.length === 0;
  let value: unknown;
  if (st.SuccessValue) {
    const raw = new TextDecoder().decode(Uint8Array.from(atob(st.SuccessValue), (c) => c.charCodeAt(0)));
    try {
      value = JSON.parse(raw);
    } catch {
      value = raw;
    }
  }
  return {
    hash: res.transaction_outcome?.id ?? res.transaction?.hash,
    ok,
    error: ok ? undefined : JSON.stringify(st.Failure ?? failures[0]?.outcome.status ?? st).slice(0, 800),
    value,
    logs,
  };
}

export interface PlacedOrder {
  orderId: string | null;
  tx: TxOutcome;
  market: string;
  side: Side;
  drafts: LotDraft[];
}

export interface BlindbookOptions {
  network: Network;
  /** NEAR account id. Defaults to the implicit account of the key. */
  accountId?: string;
  /** `ed25519:<base58>` secret key as stored by near-cli. */
  secretKey: string;
}

export class Blindbook {
  readonly network: Network;
  readonly accountId: string;
  readonly publicKey: string;
  private readonly seed: Uint8Array;
  private readonly account: Account;

  constructor(opts: BlindbookOptions) {
    this.network = opts.network;
    this.seed = seedFromNearKey(opts.secretKey);
    this.accountId = opts.accountId ?? implicitAccountOf(this.seed);
    this.publicKey = publicKeyOf(this.seed);
    const provider = new JsonRpcProvider({ url: opts.network.rpc });
    this.account = new Account(this.accountId, provider, KeyPairSigner.fromSecretKey(opts.secretKey as any));
  }

  market(id: string): Market {
    const m = this.network.markets.find((x) => x.id === id);
    if (!m) throw new Error(`unknown market ${id}`);
    return m;
  }

  async call(contract: string, method: string, args: object, gasT = 100, deposit = 0n): Promise<TxOutcome> {
    const res = await this.account.signAndSendTransaction({
      receiverId: contract,
      actions: [actions.functionCall(method, args, BigInt(gasT) * TGAS, deposit)],
      waitUntil: "FINAL" as any,
      throwOnFailure: false,
    } as any);
    return summarize(res);
  }

  view<T = unknown>(contract: string, method: string, args: object = {}): Promise<T> {
    return rpcView<T>(this.network.rpc, contract, method, args);
  }

  // ---------- verifier ----------

  /** Named accounts must register their signing key with the verifier once. */
  async ensureVerifierKey(): Promise<TxOutcome | null> {
    if (/^[0-9a-f]{64}$/.test(this.accountId)) return null;
    const has = await this.view<boolean>(this.network.verifier, "has_public_key", {
      account_id: this.accountId,
      public_key: this.publicKey,
    });
    if (has) return null;
    return this.call(this.network.verifier, "add_public_key", { public_key: this.publicKey }, 30, ONE_YOCTO);
  }

  balance(token: string): Promise<bigint> {
    return balanceOf(this.network.rpc, this.network.verifier, this.accountId, token);
  }

  /** Move NEP-141 tokens the account holds into its verifier balance. */
  async deposit(tokenContract: string, amount: bigint): Promise<TxOutcome> {
    return this.call(
      tokenContract,
      "ft_transfer_call",
      { receiver_id: this.network.verifier, amount: amount.toString(), msg: "" },
      100,
      ONE_YOCTO,
    );
  }

  // ---------- orders ----------

  async matcherKey(): Promise<string> {
    const key = await this.view<string | null>(this.network.pool, "get_matcher_key");
    if (!key) throw new Error("the pool has not published a matcher key yet");
    return key;
  }

  /**
   * Draft, sign and seal an order, then submit it to the pool. `limits` holds
   * one quote limit per lot (smallest units).
   */
  async placeOrder(opts: {
    market: string;
    side: Side;
    limits: bigint[];
    minLots?: number;
    ttlMinutes?: number;
    depositYocto?: bigint;
  }): Promise<PlacedOrder> {
    const market = this.market(opts.market);
    const salt = await fetchSalt(this.network.rpc, this.network.verifier);
    const deadline = new Date(Date.now() + (opts.ttlMinutes ?? 60) * 60_000);
    const drafts = draftLots({
      market,
      side: opts.side,
      signerId: this.accountId,
      verifier: this.network.verifier,
      salt,
      limits: opts.limits,
      deadline,
    });
    const body = orderBody({
      market,
      owner: this.accountId,
      side: opts.side,
      minLots: opts.minLots,
      lots: drafts.map((draft) => ({ draft, signed: signNep413(draft.payload, this.seed) })),
    });
    const sealed = sealOrderBody(await this.matcherKey(), body);
    const sealedLen = Math.floor((sealed.length * 3) / 4) - (sealed.endsWith("==") ? 2 : sealed.endsWith("=") ? 1 : 0);
    const deposit =
      opts.depositYocto ?? BigInt(await this.view<string>(this.network.pool, "get_order_deposit", { bytes: sealedLen }));
    // the deposit covers the order's storage and is refunded once the matcher ingests it
    const tx = await this.call(this.network.pool, "submit_order", { market: market.id, sealed_b64: sealed }, 50, deposit);
    const orderId = tx.ok && tx.value != null ? String(tx.value) : null;
    return { orderId, tx, market: market.id, side: opts.side, drafts };
  }

  /** Which lots of an order have been consumed (filled or cancelled). */
  async lotStatus(drafts: LotDraft[]): Promise<boolean[]> {
    return Promise.all(
      drafts.map((d) => isNonceUsed(this.network.rpc, this.network.verifier, this.accountId, d.payload.nonce)),
    );
  }

  /**
   * Cancel every unconsumed lot by executing cancel tickets on the verifier.
   * This needs no cooperation from the venue.
   */
  async cancel(drafts: LotDraft[]): Promise<TxOutcome | null> {
    const used = await this.lotStatus(drafts);
    const open = drafts.filter((_, i) => !used[i]);
    if (open.length === 0) return null;
    const signed: MultiPayload[] = cancelTickets(open).map((p) => signNep413(p, this.seed));
    return this.call(this.network.verifier, "execute_intents", { signed }, 100);
  }

  /** Pool configuration, including the gas and deposit a crank needs. */
  poolConfig(): Promise<PoolConfig> {
    return this.view<PoolConfig>(this.network.pool, "get_config");
  }

  /** Anyone may crank a batch; the unused OutLayer deposit is refunded. */
  async runBatch(): Promise<TxOutcome> {
    const cfg = await this.poolConfig();
    const gasT = Math.ceil(Number(BigInt(cfg.run_gas) / TGAS)) + 15;
    return this.call(this.network.pool, "run_batch", {}, gasT, BigInt(cfg.run_deposit));
  }
}

export interface PoolConfig {
  owner: string;
  network: string;
  project_id: string;
  version_key: string;
  result_key: string | null;
  matcher_key: string | null;
  venue_account: string;
  verifier: string;
  params: Record<string, number>;
  run_deposit: string;
  run_gas: string;
  pending: number;
  next_batch_id: number;
}

export * from "./verify.js";
export type { LotDraft, Market, Network, Side };
