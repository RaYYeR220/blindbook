import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { TESTNET } from "@blindbook/sdk";
import { describe, expect, it } from "vitest";
import { Blindbook } from "../src/index.js";

const keyFile = join(homedir(), ".near-credentials", "testnet", "seller-bbcced2c.testnet.json");

describe.skipIf(!existsSync(keyFile) || !process.env.BLINDBOOK_LIVE)("testnet read path", () => {
  it("reads verifier key registration and balances", async () => {
    const { private_key } = JSON.parse(readFileSync(keyFile, "utf8"));
    const bb = new Blindbook({ network: TESTNET, accountId: "seller-bbcced2c.testnet", secretKey: private_key });
    const has = await bb.view<boolean>(TESTNET.verifier, "has_public_key", { account_id: bb.accountId, public_key: bb.publicKey });
    expect(has).toBe(true);
    const bal = await bb.balance(TESTNET.markets[0].base);
    expect(bal).toBeGreaterThanOrEqual(0n);
  }, 30_000);
});
