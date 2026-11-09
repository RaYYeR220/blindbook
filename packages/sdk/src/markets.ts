import type { Market } from "./order.js";

export const MAINNET_USDC = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
export const MAINNET_WNEAR = "nep141:wrap.near";

export interface Network {
  networkId: "mainnet" | "testnet";
  rpc: string;
  verifier: string;
  pool: string;
  markets: Market[];
}

export const MAINNET: Network = {
  networkId: "mainnet",
  rpc: "https://rpc.mainnet.fastnear.com",
  verifier: "intents.near",
  pool: "pool.blindbook.near",
  markets: [
    {
      id: "wnear-usdc",
      base: MAINNET_WNEAR,
      quote: MAINNET_USDC,
      lot: 100_000_000_000_000_000_000_000n, // 0.1 NEAR
      baseDecimals: 24,
      quoteDecimals: 6,
      baseSymbol: "NEAR",
      quoteSymbol: "USDC",
    },
  ],
};

export const TESTNET: Network = {
  networkId: "testnet",
  rpc: "https://test.rpc.fastnear.com",
  verifier: "intents-bbcced2c.testnet",
  pool: "pool.blindbook-dev.testnet",
  markets: [
    {
      id: "tnear-tusdc",
      base: "nep141:tnear-bbcced2c.testnet",
      quote: "nep141:tusdc-bbcced2c.testnet",
      lot: 1_000_000_000_000_000_000_000_000n, // 1 tNEAR
      baseDecimals: 24,
      quoteDecimals: 6,
      baseSymbol: "tNEAR",
      quoteSymbol: "tUSDC",
    },
  ],
};
