import "server-only";
import { MAINNET, type Network, TESTNET } from "@blindbook/sdk";

export const network: Network = process.env.NEXT_PUBLIC_BLINDBOOK_NETWORK === "mainnet" ? MAINNET : TESTNET;

export const explorer =
  network.networkId === "mainnet" ? "https://nearblocks.io/txns/" : "https://testnet.nearblocks.io/txns/";

export const outlayerApi =
  network.networkId === "mainnet" ? "https://api.outlayer.ai" : "https://testnet-api.outlayer.ai";

/** Server-held keys for the public demo. Both are capped by the demo flow. */
export function demoKeys() {
  const demo = { accountId: process.env.DEMO_ACCOUNT_ID, secretKey: process.env.DEMO_SECRET_KEY };
  const house = { accountId: process.env.HOUSE_ACCOUNT_ID, secretKey: process.env.HOUSE_SECRET_KEY };
  if (!demo.accountId || !demo.secretKey || !house.accountId || !house.secretKey) return null;
  return {
    demo: demo as { accountId: string; secretKey: string },
    house: house as { accountId: string; secretKey: string },
  };
}
