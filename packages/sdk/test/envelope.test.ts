import { readFileSync } from "node:fs";
import { sha256 } from "@noble/hashes/sha256";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils";
import { describe, expect, it } from "vitest";
import { BUCKETS, sealOrder } from "../src/envelope.js";

const vector = JSON.parse(
  readFileSync(new URL("../../../crates/core/tests/fixtures/envelope_vector.json", import.meta.url), "utf8"),
);

describe("sealOrder", () => {
  it("matches the Rust matcher byte for byte", () => {
    const sealed = sealOrder(hexToBytes(vector.matcherPublicKey), vector.body, {
      ephemeralSecret: hexToBytes(vector.ephemeralSecret),
      nonce: hexToBytes(vector.nonce),
    });
    expect(sealed.length).toBe(vector.sealedLength);
    expect(bytesToHex(sha256(sealed))).toBe(vector.sealedSha256);
  });

  it("hides the body size inside a bucket", () => {
    const pk = hexToBytes(vector.matcherPublicKey);
    expect(sealOrder(pk, "a").length).toBe(sealOrder(pk, "a".repeat(1_500)).length);
    expect(sealOrder(pk, "a".repeat(3_000)).length).toBe(57 + BUCKETS[1] + 16);
  });

  it("uses fresh randomness by default", () => {
    const pk = hexToBytes(vector.matcherPublicKey);
    expect(bytesToHex(sealOrder(pk, "x"))).not.toBe(bytesToHex(sealOrder(pk, "x")));
  });
});
