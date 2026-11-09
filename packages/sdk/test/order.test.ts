import { writeFileSync } from "node:fs";
import { secp256k1 } from "@noble/curves/secp256k1";
import { keccak_256 } from "@noble/hashes/sha3";
import { bytesToHex } from "@noble/hashes/utils";
import { describe, expect, it } from "vitest";
import {
  cancelTickets,
  decodeNonce,
  draftLots,
  erc191FromSignature,
  erc191Message,
  evmAccountId,
  implicitAccountOf,
  limitForLot,
  MAINNET,
  makeNonce,
  nep413FromSignature,
  orderBody,
  priceOfLot,
  publicKeyOf,
  signNep413,
} from "../src/index.js";

const market = MAINNET.markets[0];
const seed = new Uint8Array(32).fill(5);

function personalSign(message: string, priv: Uint8Array): string {
  const m = new TextEncoder().encode(message);
  const prefix = new TextEncoder().encode(`\x19Ethereum Signed Message:\n${m.length}`);
  const digest = keccak_256(new Uint8Array([...prefix, ...m]));
  const sig = secp256k1.sign(digest, priv);
  return `0x${sig.toCompactHex()}${(27 + sig.recovery).toString(16)}`;
}

function evmAddress(priv: Uint8Array): string {
  const pub = secp256k1.getPublicKey(priv, false).slice(1);
  return `0x${bytesToHex(keccak_256(pub).slice(12))}`;
}

describe("prices and lots", () => {
  it("converts a price into a per-lot limit and back", () => {
    // 0.1 NEAR at 3.54 USDC/NEAR = 0.354 USDC = 354000 units
    expect(limitForLot(market, "3.54")).toBe(354_000n);
    expect(priceOfLot(market, 354_000n)).toBe("3.54");
    expect(limitForLot(market, "3")).toBe(300_000n);
  });

  it("drafts one token_diff per lot with distinct versioned nonces", () => {
    const deadline = new Date("2026-11-12T00:00:00Z");
    const lots = draftLots({
      market,
      side: "sell",
      signerId: "alice.near",
      verifier: "intents.near",
      salt: "252812b3",
      limits: [354_000n, 355_000n, 356_000n],
      deadline,
    });
    expect(lots).toHaveLength(3);
    expect(new Set(lots.map((l) => l.payload.nonce)).size).toBe(3);
    const first = lots[0].payload;
    expect(first.intents[0]).toEqual({
      intent: "token_diff",
      diff: { [market.base]: "-100000000000000000000000", [market.quote]: "354000" },
    });
    const n = decodeNonce(first.nonce);
    expect(n.legacy).toBe(false);
    if (!n.legacy) {
      expect(n.salt).toBe("252812b3");
      expect(n.deadline.toISOString()).toBe(deadline.toISOString());
    }
  });

  it("buys pay quote and receive one lot", () => {
    const [lot] = draftLots({
      market,
      side: "buy",
      signerId: "bob.near",
      verifier: "intents.near",
      salt: "252812b3",
      limits: [360_000n],
      deadline: new Date("2026-11-12T00:00:00Z"),
    });
    expect(lot.payload.intents[0]).toMatchObject({ diff: { [market.base]: "100000000000000000000000", [market.quote]: "-360000" } });
  });

  it("cancel tickets keep the nonce and carry no intents", () => {
    const lots = draftLots({
      market,
      side: "sell",
      signerId: "alice.near",
      verifier: "intents.near",
      salt: "252812b3",
      limits: [354_000n, 354_000n],
      deadline: new Date("2026-11-12T00:00:00Z"),
    });
    const tickets = cancelTickets(lots);
    expect(tickets.map((t) => t.nonce)).toEqual(lots.map((l) => l.payload.nonce));
    expect(tickets.every((t) => t.intents.length === 0)).toBe(true);
  });
});

describe("signing", () => {
  it("reads the nonce layout the verifier uses", () => {
    // a cancel ticket executed on the testnet verifier during the settlement spike
    const n = decodeNonce("Vij2xgANkyeyQAGoTi5j5hhJDPy0dENf60gFDSmL68s=");
    expect(n.legacy).toBe(false);
    if (!n.legacy) expect(n.salt).toBe("0d9327b2");
  });

  it("produces NEP-413 payloads a wallet signature can be checked against", () => {
    const [lot] = draftLots({
      market,
      side: "sell",
      signerId: implicitAccountOf(seed),
      verifier: "intents.near",
      salt: "252812b3",
      limits: [354_000n],
      deadline: new Date("2026-11-12T00:00:00Z"),
    });
    const signed = signNep413(lot.payload, seed);
    expect(signed.standard).toBe("nep413");
    if (signed.standard !== "nep413") return;
    expect(signed.public_key).toBe(publicKeyOf(seed));
    // a signature for a different payload is refused before it reaches the matcher
    const other = { ...lot.payload, deadline: "2026-11-13T00:00:00.000Z" };
    expect(() => nep413FromSignature(other, signed.public_key, signed.signature)).toThrow();
  });

  it("normalizes ERC-191 recovery ids to 0 or 1", () => {
    const priv = new Uint8Array(32).fill(9);
    const [lot] = draftLots({
      market,
      side: "buy",
      signerId: evmAccountId(evmAddress(priv)),
      verifier: "intents.near",
      salt: "252812b3",
      limits: [360_000n],
      deadline: new Date("2026-11-12T00:00:00Z"),
    });
    const signed = erc191FromSignature(lot.payload, personalSign(erc191Message(lot.payload), priv));
    expect(signed.standard).toBe("erc191");
    if (signed.standard === "erc191") expect(signed.signature.startsWith("secp256k1:")).toBe(true);
  });
});

describe("interop fixture for the Rust matcher", () => {
  it("writes deterministic signed lots that blindbook-core validates", () => {
    const deadline = new Date("2030-01-01T00:00:00Z");
    const fixedNonce = (i: number) => makeNonce("252812b3", deadline, new Uint8Array(15).fill(i + 1));
    const nearOwner = implicitAccountOf(seed);
    const sells = [354_000n, 355_000n].map((limitQuote, index) => {
      const payload = { ...draftLots({ market, side: "sell", signerId: nearOwner, verifier: "intents.near", salt: "252812b3", limits: [limitQuote], deadline })[0].payload, nonce: fixedNonce(index) };
      return { draft: { index, limitQuote, payload }, signed: signNep413(payload, seed) };
    });
    const priv = new Uint8Array(32).fill(9);
    const evmOwner = evmAccountId(evmAddress(priv));
    const buys = [360_000n].map((limitQuote, index) => {
      const payload = { ...draftLots({ market, side: "buy", signerId: evmOwner, verifier: "intents.near", salt: "252812b3", limits: [limitQuote], deadline })[0].payload, nonce: fixedNonce(10 + index) };
      return { draft: { index, limitQuote, payload }, signed: erc191FromSignature(payload, personalSign(erc191Message(payload), priv)) };
    });
    const fixture = {
      market: { id: market.id, base: market.base, quote: market.quote, lot: market.lot.toString() },
      verifier: "intents.near",
      sell: orderBody({ market, owner: nearOwner, side: "sell", lots: sells }),
      buy: orderBody({ market, owner: evmOwner, side: "buy", lots: buys }),
    };
    const path = new URL("../../../crates/core/tests/fixtures/sdk_orders.json", import.meta.url);
    writeFileSync(path, `${JSON.stringify(fixture, null, 2)}\n`);
    expect(fixture.sell.lots).toHaveLength(2);
  });
});
