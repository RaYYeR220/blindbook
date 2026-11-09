/**
 * NEAR Intents verifier payloads: building, versioned nonces, NEP-413 and
 * ERC-191 signing, cancel tickets and read-only RPC.
 *
 * Checked against the verifier running on `intents.near` (defuse 0.4.5).
 * Browser-safe: no Node built-ins.
 */
import { ed25519 } from "@noble/curves/ed25519";
import { sha256 } from "@noble/hashes/sha256";
import { keccak_256 } from "@noble/hashes/sha3";
import { base58, base64, hex } from "@scure/base";

export type TokenId = string;
export type Amounts = Record<TokenId, string>;

export type Intent =
  | { intent: "token_diff"; diff: Amounts; memo?: string; referral?: string }
  | { intent: "transfer"; receiver_id: string; tokens: Amounts; memo?: string }
  | { intent: "add_public_key"; public_key: string }
  | { intent: "remove_public_key"; public_key: string }
  | { intent: "ft_withdraw"; token: string; receiver_id: string; amount: string; memo?: string; msg?: string };

export interface IntentPayload {
  signer_id: string;
  verifying_contract: string;
  /** RFC 3339, e.g. 2026-11-09T14:00:00.000Z */
  deadline: string;
  /** base64 of 32 bytes */
  nonce: string;
  intents: Intent[];
}

export type MultiPayload =
  | {
      standard: "nep413";
      payload: { message: string; nonce: string; recipient: string; callbackUrl?: string };
      public_key: string;
      signature: string;
    }
  | { standard: "erc191"; payload: string; signature: string };

const te = new TextEncoder();

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}
const u32le = (n: number) => {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n >>> 0, true);
  return b;
};
const borshString = (s: string) => {
  const b = te.encode(s);
  return concat(u32le(b.length), b);
};
const randomBytes = (n: number) => crypto.getRandomValues(new Uint8Array(n));

// ---------- versioned nonces ----------

/** First 4 bytes of sha256("versioned_nonce"). */
export const VERSIONED_NONCE_MAGIC = "5628f6c6";

/**
 * Versioned nonce: magic (4) | version 0 (1) | salt (4) | nonce deadline,
 * i64 ns LE (8) | random (15). The verifier requires a known salt, an intent
 * deadline no later than the nonce deadline, and a nonce deadline in the future.
 */
export function makeNonce(saltHex: string, nonceDeadline: Date, random15: Uint8Array = randomBytes(15)): string {
  const salt = hex.decode(saltHex);
  if (salt.length !== 4) throw new Error("salt must be 4 bytes of hex");
  if (random15.length !== 15) throw new Error("random part must be 15 bytes");
  const dl = new Uint8Array(8);
  new DataView(dl.buffer).setBigInt64(0, BigInt(nonceDeadline.getTime()) * 1_000_000n, true);
  return base64.encode(concat(hex.decode(VERSIONED_NONCE_MAGIC), new Uint8Array([0]), salt, dl, random15));
}

export function decodeNonce(nonceB64: string) {
  const b = base64.decode(nonceB64);
  if (b.length !== 32) throw new Error("nonce must be 32 bytes");
  if (hex.encode(b.subarray(0, 4)) !== VERSIONED_NONCE_MAGIC) return { legacy: true as const };
  const ns = new DataView(b.buffer, b.byteOffset + 9, 8).getBigInt64(0, true);
  return {
    legacy: false as const,
    version: b[4],
    salt: hex.encode(b.subarray(5, 9)),
    deadline: new Date(Number(ns / 1_000_000n)),
  };
}

// ---------- payloads ----------

const str = (v: bigint | string | number) => v.toString();

export function tokenDiff(diff: Record<TokenId, bigint | string | number>): Intent {
  return { intent: "token_diff", diff: Object.fromEntries(Object.entries(diff).map(([k, v]) => [k, str(v)])) };
}

export function transfer(receiverId: string, tokens: Record<TokenId, bigint | string | number>): Intent {
  return {
    intent: "transfer",
    receiver_id: receiverId,
    tokens: Object.fromEntries(Object.entries(tokens).map(([k, v]) => [k, str(v)])),
  };
}

export function buildPayload(p: {
  signerId: string;
  verifier: string;
  deadline: Date;
  nonce: string;
  intents: Intent[];
}): IntentPayload {
  return {
    signer_id: p.signerId,
    verifying_contract: p.verifier,
    deadline: p.deadline.toISOString(),
    nonce: p.nonce,
    intents: p.intents,
  };
}

/**
 * A cancel ticket: the same signer and nonce with no intents. Executing it on
 * the verifier consumes the nonce, so the original payload can never run.
 */
export function cancelTicket(payload: IntentPayload): IntentPayload {
  return { ...payload, intents: [] };
}

// ---------- NEP-413 (NEAR wallets and keys) ----------

/** 2^31 + 413 */
export const NEP413_TAG = 2_147_484_061;

export function nep413Hash(message: string, nonce: Uint8Array, recipient: string, callbackUrl?: string): Uint8Array {
  const cb = callbackUrl == null ? new Uint8Array([0]) : concat(new Uint8Array([1]), borshString(callbackUrl));
  return sha256(concat(u32le(NEP413_TAG), borshString(message), nonce, borshString(recipient), cb));
}

/** What a NEAR wallet's `signMessage` must be asked to sign for this payload. */
export function nep413Request(payload: IntentPayload): { message: string; recipient: string; nonce: Uint8Array } {
  return {
    message: JSON.stringify({ signer_id: payload.signer_id, deadline: payload.deadline, intents: payload.intents }),
    recipient: payload.verifying_contract,
    nonce: base64.decode(payload.nonce),
  };
}

/** Wrap a wallet's NEP-413 signature (base64 or `ed25519:` base58) as a verifier payload. */
export function nep413FromSignature(payload: IntentPayload, publicKey: string, signature: string | Uint8Array): MultiPayload {
  const req = nep413Request(payload);
  const sig =
    typeof signature === "string"
      ? signature.startsWith("ed25519:")
        ? base58.decode(signature.slice(8))
        : base64.decode(signature)
      : signature;
  const pk = base58.decode(publicKey.replace(/^ed25519:/, ""));
  if (!ed25519.verify(sig, nep413Hash(req.message, req.nonce, req.recipient), pk)) {
    throw new Error("NEP-413 signature does not match the payload");
  }
  return {
    standard: "nep413",
    payload: { message: req.message, nonce: payload.nonce, recipient: req.recipient },
    public_key: publicKey.startsWith("ed25519:") ? publicKey : `ed25519:${publicKey}`,
    signature: `ed25519:${base58.encode(sig)}`,
  };
}

/** Sign with a raw 32-byte ed25519 seed (agents, scripts). */
export function signNep413(payload: IntentPayload, seed: Uint8Array): MultiPayload {
  const req = nep413Request(payload);
  const sig = ed25519.sign(nep413Hash(req.message, req.nonce, req.recipient), seed);
  return nep413FromSignature(payload, `ed25519:${base58.encode(ed25519.getPublicKey(seed))}`, sig);
}

/** `ed25519:<base58 64-byte secret>` as stored by near-cli -> 32-byte seed. */
export function seedFromNearKey(secret: string): Uint8Array {
  return base58.decode(secret.replace(/^ed25519:/, "")).slice(0, 32);
}

export const publicKeyOf = (seed: Uint8Array) => `ed25519:${base58.encode(ed25519.getPublicKey(seed))}`;

/** Implicit NEAR account of an ed25519 key: hex of the public key. */
export const implicitAccountOf = (seed: Uint8Array) => hex.encode(ed25519.getPublicKey(seed));

// ---------- ERC-191 (EVM wallets) ----------

/** The exact string an EVM wallet must `personal_sign`. */
export function erc191Message(payload: IntentPayload): string {
  return JSON.stringify({
    signer_id: payload.signer_id,
    verifying_contract: payload.verifying_contract,
    deadline: payload.deadline,
    nonce: payload.nonce,
    intents: payload.intents,
  });
}

/** Wrap a `personal_sign` signature (0x-hex, 65 bytes) as a verifier payload. */
export function erc191FromSignature(payload: IntentPayload, signatureHex: string): MultiPayload {
  const sig = hex.decode(signatureHex.replace(/^0x/, ""));
  if (sig.length !== 65) throw new Error("expected a 65-byte signature");
  // The verifier accepts only a normalized recovery id.
  if (sig[64] >= 27) sig[64] -= 27;
  if (sig[64] > 1) throw new Error("invalid recovery id");
  return { standard: "erc191", payload: erc191Message(payload), signature: `secp256k1:${base58.encode(sig)}` };
}

/** Lowercase `0x` address: the implicit verifier account of an EVM key. */
export const evmAccountId = (address: string) => address.toLowerCase();

// ---------- hashes ----------

/** Hash the verifier reports for an executed payload in `intents_executed`. */
export function intentHash(mp: MultiPayload): string {
  if (mp.standard === "nep413") {
    return base58.encode(
      nep413Hash(mp.payload.message, base64.decode(mp.payload.nonce), mp.payload.recipient, mp.payload.callbackUrl),
    );
  }
  const m = te.encode(mp.payload);
  return base58.encode(keccak_256(concat(te.encode(`\x19Ethereum Signed Message:\n${m.length}`), m)));
}

// ---------- read-only RPC ----------

export async function rpcView<T = unknown>(rpcUrl: string, contract: string, method: string, args: unknown = {}): Promise<T> {
  const res = await fetch(rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: "blindbook",
      method: "query",
      params: {
        request_type: "call_function",
        finality: "final",
        account_id: contract,
        method_name: method,
        args_base64: base64.encode(te.encode(JSON.stringify(args))),
      },
    }),
  });
  const j = (await res.json()) as { error?: unknown; result?: { error?: string; result?: number[] } };
  if (j.error) throw new Error(`RPC error: ${JSON.stringify(j.error)}`);
  if (j.result?.error) throw new Error(j.result.error);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(j.result!.result!))) as T;
}

export const fetchSalt = (rpcUrl: string, verifier: string) => rpcView<string>(rpcUrl, verifier, "current_salt");

export const fetchFeePips = (rpcUrl: string, verifier: string) => rpcView<number>(rpcUrl, verifier, "fee");

export const isNonceUsed = (rpcUrl: string, verifier: string, accountId: string, nonce: string) =>
  rpcView<boolean>(rpcUrl, verifier, "is_nonce_used", { account_id: accountId, nonce });

export const balanceOf = (rpcUrl: string, verifier: string, accountId: string, tokenId: string) =>
  rpcView<string>(rpcUrl, verifier, "mt_balance_of", { account_id: accountId, token_id: tokenId }).then(BigInt);
