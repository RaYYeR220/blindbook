/**
 * Sealed order envelopes, byte-compatible with `blindbook-core::envelope`.
 *
 * Layout: version (1) || ephemeral public key (32) || nonce (24) || aead.
 * Key: HKDF-SHA256(ikm = X25519(eph, matcher), salt = eph_pub || matcher_pub,
 * info = "blindbook/v1/order"). The plaintext is padded to a fixed bucket so
 * the ciphertext length does not reveal how many lots an order carries.
 */
import { xchacha20poly1305 } from "@noble/ciphers/chacha";
import { x25519 } from "@noble/curves/ed25519";
import { hkdf } from "@noble/hashes/hkdf";
import { sha256 } from "@noble/hashes/sha256";
import { randomBytes } from "@noble/hashes/utils";

export const ENVELOPE_VERSION = 1;
export const BUCKETS = [2_048, 4_096, 8_192, 16_384, 32_768, 65_536] as const;
const INFO = new TextEncoder().encode("blindbook/v1/order");

function pad(body: Uint8Array): Uint8Array {
  const need = body.length + 4;
  const size = BUCKETS.find((b) => b >= need);
  if (size === undefined) throw new Error(`order body of ${body.length} bytes exceeds the largest bucket`);
  const out = new Uint8Array(size);
  new DataView(out.buffer).setUint32(0, body.length, true);
  out.set(body, 4);
  return out;
}

function deriveKey(shared: Uint8Array, ephPub: Uint8Array, matcherPub: Uint8Array): Uint8Array {
  const salt = new Uint8Array(64);
  salt.set(ephPub, 0);
  salt.set(matcherPub, 32);
  return hkdf(sha256, shared, salt, INFO, 32);
}

export interface SealOptions {
  /** Fixed ephemeral secret, for test vectors only. */
  ephemeralSecret?: Uint8Array;
  /** Fixed nonce, for test vectors only. */
  nonce?: Uint8Array;
}

/** Encrypt an order body to the matcher's X25519 public key. */
export function sealOrder(matcherPublicKey: Uint8Array, body: Uint8Array | string, opts: SealOptions = {}): Uint8Array {
  if (matcherPublicKey.length !== 32) throw new Error("matcher public key must be 32 bytes");
  const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
  const eph = opts.ephemeralSecret ?? randomBytes(32);
  const nonce = opts.nonce ?? randomBytes(24);
  const ephPub = x25519.getPublicKey(eph);
  const shared = x25519.getSharedSecret(eph, matcherPublicKey);
  const key = deriveKey(shared, ephPub, matcherPublicKey);
  const header = new Uint8Array(33);
  header[0] = ENVELOPE_VERSION;
  header.set(ephPub, 1);
  const ct = xchacha20poly1305(key, nonce, header).encrypt(pad(bytes));
  const out = new Uint8Array(33 + 24 + ct.length);
  out.set(header, 0);
  out.set(nonce, 33);
  out.set(ct, 57);
  return out;
}
