import { balanceOf, rpcView } from "@blindbook/sdk";
import type { NextRequest } from "next/server";
import { network } from "@/lib/server/env";

/** An account's verifier balances for every market token, and whether a key is registered. */
export async function GET(request: NextRequest) {
  const account = request.nextUrl.searchParams.get("account")?.toLowerCase();
  const key = request.nextUrl.searchParams.get("key");
  if (!account) return Response.json({ error: "account required" }, { status: 400 });
  const tokens = [...new Set(network.markets.flatMap((m) => [m.base, m.quote]))];
  const balances: Record<string, string> = {};
  for (const t of tokens) balances[t] = (await balanceOf(network.rpc, network.verifier, account, t).catch(() => 0n)).toString();
  const implicit = /^[0-9a-f]{64}$/.test(account) || /^0x[0-9a-f]{40}$/.test(account);
  const keyRegistered = implicit
    ? true
    : key
      ? await rpcView<boolean>(network.rpc, network.verifier, "has_public_key", { account_id: account, public_key: key }).catch(() => false)
      : null;
  return Response.json({ account, balances, keyRegistered, verifier: network.verifier });
}
