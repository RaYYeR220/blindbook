import {
  erc191FromSignature,
  erc191Message,
  evmAccountId,
  type IntentPayload,
  type MultiPayload,
  nep413FromSignature,
  nep413Request,
} from "@blindbook/sdk";
import type { ConnectorAction, NearConnector, NearWalletBase } from "@hot-labs/near-connect";
import { NET } from "./net";
import { readJson, removeKey, writeJson } from "./storage";

/**
 * Two ways to sign lots, behind one interface:
 *   - NEAR wallets through @hot-labs/near-connect, signing NEP-413 messages;
 *   - EVM wallets through window.ethereum, signing ERC-191 personal messages.
 * The app only ever asks a wallet to sign; it never sees or stores a key.
 */

export type WalletKind = "near" | "evm";

export interface Signed {
  signed: MultiPayload;
  /** The NEAR key that signed, `ed25519:...`; null for EVM signatures. */
  publicKey: string | null;
}

export interface Wallet {
  kind: WalletKind;
  /** The verifier account: a NEAR account id, or the lowercase 0x address. */
  accountId: string;
  /** The NEAR signing key when the wallet reports it before the first signature. */
  publicKey: string | null;
  name: string;
  standard: "NEP-413" | "ERC-191";
  sign(payload: IntentPayload): Promise<Signed>;
  /** Register a NEAR signing key with the verifier: one `add_public_key` call with 1 yoctoNEAR. Returns the tx hash. */
  registerKey?(publicKey: string): Promise<string>;
  disconnect(): Promise<void>;
}

/** Named NEAR accounts must register their signing key on the verifier once; implicit and 0x accounts need not. */
export const needsKeyRegistration = (w: Pick<Wallet, "kind" | "accountId">) =>
  w.kind === "near" && !/^[0-9a-f]{64}$/.test(w.accountId);

const SESSION_KEY = "bb:wallet:v1";
type Session = { kind: WalletKind; accountId: string };

/** A wallet rejection reads differently from a failure: the person said no. */
export function walletErrorMessage(e: unknown): string {
  const err = e as { message?: string; code?: number } | null;
  const msg = err?.message ?? String(e);
  if (err?.code === 4001 || /reject|denied|cancel|closed|user/i.test(msg)) return "The wallet request was declined.";
  return msg.length > 220 ? `${msg.slice(0, 220)}…` : msg;
}

// ---------------------------------------------------------------- NEAR

let connector: Promise<NearConnector> | null = null;

function nearConnector(): Promise<NearConnector> {
  connector ??= import("@hot-labs/near-connect").then(
    ({ NearConnector: Connector }) =>
      new Connector({
        network: NET.networkId,
        features: NET.networkId === "testnet" ? { signMessage: true, testnet: true } : { signMessage: true, mainnet: true },
      }),
  );
  return connector;
}

function txHashOf(outcome: unknown): string {
  const o = outcome as { transaction_outcome?: { id?: string }; transaction?: { hash?: string } } | null;
  return o?.transaction_outcome?.id ?? o?.transaction?.hash ?? "";
}

function nearWallet(wallet: NearWalletBase, accountId: string, publicKey: string | null): Wallet {
  const network = NET.networkId;
  return {
    kind: "near",
    accountId,
    publicKey,
    name: wallet.manifest.name,
    standard: "NEP-413",
    async sign(payload) {
      const req = nep413Request(payload);
      const res = await wallet.signMessage({
        message: req.message,
        recipient: req.recipient,
        nonce: req.nonce,
        network,
        signerId: accountId,
      });
      if (res.accountId && res.accountId !== accountId) {
        throw new Error(`the wallet signed as ${res.accountId}, not ${accountId}`);
      }
      return { signed: nep413FromSignature(payload, res.publicKey, res.signature), publicKey: res.publicKey };
    },
    async registerKey(key) {
      const action: ConnectorAction = {
        type: "FunctionCall",
        params: { methodName: "add_public_key", args: { public_key: key }, gas: "30000000000000", deposit: "1" },
      };
      const outcome = await wallet.signAndSendTransaction({ network, signerId: accountId, receiverId: NET.verifier, actions: [action] });
      return txHashOf(outcome);
    },
    async disconnect() {
      removeKey(SESSION_KEY);
      const c = await nearConnector();
      await c.disconnect(wallet).catch(() => undefined);
    },
  };
}

export async function connectNear(): Promise<Wallet> {
  const c = await nearConnector();
  const wallet = await c.connect();
  const accounts = await wallet.getAccounts({ network: NET.networkId });
  const account = accounts[0];
  if (!account) throw new Error("the wallet returned no account");
  writeJson(SESSION_KEY, { kind: "near", accountId: account.accountId } satisfies Session);
  return nearWallet(wallet, account.accountId, account.publicKey ?? null);
}

// ---------------------------------------------------------------- EVM

interface Eip1193 {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
  on?(event: "accountsChanged", cb: (accounts: string[]) => void): void;
  removeListener?(event: "accountsChanged", cb: (accounts: string[]) => void): void;
}

const ethereum = (): Eip1193 | null =>
  typeof window === "undefined" ? null : ((window as unknown as { ethereum?: Eip1193 }).ethereum ?? null);

export const hasEvmWallet = () => ethereum() !== null;

const utf8Hex = (s: string) =>
  `0x${Array.from(new TextEncoder().encode(s), (b) => b.toString(16).padStart(2, "0")).join("")}`;

function evmWallet(eth: Eip1193, address: string): Wallet {
  const accountId = evmAccountId(address);
  return {
    kind: "evm",
    accountId,
    publicKey: null,
    name: "EVM wallet",
    standard: "ERC-191",
    async sign(payload) {
      const sig = (await eth.request({ method: "personal_sign", params: [utf8Hex(erc191Message(payload)), address] })) as string;
      return { signed: erc191FromSignature(payload, sig), publicKey: null };
    },
    async disconnect() {
      removeKey(SESSION_KEY);
      await eth.request({ method: "wallet_revokePermissions", params: [{ eth_accounts: {} }] }).catch(() => undefined);
    },
  };
}

export async function connectEvm(): Promise<Wallet> {
  const eth = ethereum();
  if (!eth) throw new Error("No EVM wallet found in this browser.");
  const accounts = (await eth.request({ method: "eth_requestAccounts" })) as string[];
  const address = accounts[0];
  if (!address) throw new Error("the wallet returned no account");
  writeJson(SESSION_KEY, { kind: "evm", accountId: evmAccountId(address) } satisfies Session);
  return evmWallet(eth, address);
}

/** Follow the EVM wallet's account switch. Returns the unsubscribe. */
export function onEvmAccountChange(cb: (wallet: Wallet | null) => void): () => void {
  const eth = ethereum();
  if (!eth?.on) return () => {};
  const handler = (accounts: string[]) => {
    const address = accounts[0];
    if (!address) {
      removeKey(SESSION_KEY);
      cb(null);
      return;
    }
    writeJson(SESSION_KEY, { kind: "evm", accountId: evmAccountId(address) } satisfies Session);
    cb(evmWallet(eth, address));
  };
  eth.on("accountsChanged", handler);
  return () => eth.removeListener?.("accountsChanged", handler);
}

// ---------------------------------------------------------------- session

/** Reconnect silently to the wallet used last time, if it still grants access. */
export async function restoreWallet(): Promise<Wallet | null> {
  const session = readJson<Session | null>(SESSION_KEY, null);
  if (!session) return null;
  try {
    if (session.kind === "evm") {
      const eth = ethereum();
      if (!eth) return null;
      const accounts = ((await eth.request({ method: "eth_accounts" })) as string[]).map(evmAccountId);
      const i = accounts.indexOf(session.accountId);
      return i >= 0 ? evmWallet(eth, accounts[i]) : null;
    }
    const c = await nearConnector();
    const { wallet, accounts } = await c.getConnectedWallet();
    const account = accounts.find((a) => a.accountId === session.accountId) ?? accounts[0];
    return account ? nearWallet(wallet, account.accountId, account.publicKey ?? null) : null;
  } catch {
    return null;
  }
}
