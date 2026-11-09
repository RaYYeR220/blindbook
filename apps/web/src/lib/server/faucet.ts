import "server-only";
import { Blindbook } from "@blindbook/client";
import { network } from "./env";

/**
 * Testnet only: credit test tokens straight into an account's verifier
 * balance, so anyone can try the app with a fresh wallet.
 */
const grants = new Map<string, number>();
const DAY = 24 * 60 * 60 * 1000;

function minter(token: "base" | "quote"): Blindbook | null {
  const id = process.env[token === "base" ? "FAUCET_BASE_ACCOUNT_ID" : "FAUCET_QUOTE_ACCOUNT_ID"];
  const key = process.env[token === "base" ? "FAUCET_BASE_SECRET_KEY" : "FAUCET_QUOTE_SECRET_KEY"];
  if (!id || !key) return null;
  return new Blindbook({ network, accountId: id, secretKey: key });
}

export const faucetAvailable = () => network.networkId === "testnet" && minter("base") !== null && minter("quote") !== null;

const VALID_ACCOUNT = /^(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/;

export async function drip(account: string) {
  if (!faucetAvailable()) throw new Error("the faucet runs on testnet only");
  const id = account.trim().toLowerCase();
  if (id.length < 2 || id.length > 64 || !VALID_ACCOUNT.test(id)) throw new Error("not a NEAR or 0x account id");
  const last = grants.get(id) ?? 0;
  if (Date.now() - last < DAY) throw new Error("this account already got test tokens today");
  grants.set(id, Date.now());
  const market = network.markets[0];
  const amounts = { base: 5n * market.lot, quote: 20n * 10n ** BigInt(market.quoteDecimals) };
  const out: Record<string, string> = {};
  for (const side of ["base", "quote"] as const) {
    const m = minter(side)!;
    const token = (side === "base" ? market.base : market.quote).replace(/^nep141:/, "");
    const tx = await m.call(
      token,
      "ft_transfer_call",
      { receiver_id: network.verifier, amount: amounts[side].toString(), msg: id },
      100,
      1n,
    );
    if (!tx.ok) throw new Error(`could not credit ${side} token`);
    out[side] = tx.hash;
  }
  return { account: id, base: amounts.base.toString(), quote: amounts.quote.toString(), txs: out };
}
