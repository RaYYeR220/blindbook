import { MAINNET, type Market, type Network, TESTNET } from "@blindbook/sdk";

/**
 * Public, browser-side view of the deployment: the network the app trades on,
 * its one market, and the explorer and OutLayer endpoints links point to.
 */
export const NET: Network = process.env.NEXT_PUBLIC_BLINDBOOK_NETWORK === "mainnet" ? MAINNET : TESTNET;
export const MARKET: Market = NET.markets[0];
export const IS_TESTNET = NET.networkId === "testnet";
export const NETWORK_LABEL = IS_TESTNET ? "NEAR testnet" : "NEAR mainnet";

const EXPLORER = IS_TESTNET ? "https://testnet.nearblocks.io" : "https://nearblocks.io";
const OUTLAYER = IS_TESTNET ? "https://testnet-api.outlayer.ai" : "https://api.outlayer.ai";
/** Where the matcher build is published; only the testnet store exists so far. */
const WASM_STORE = IS_TESTNET ? "https://test.fastfs.io/blindbook-dev.testnet/outlayer.testnet" : null;

export const txUrl = (hash: string) => `${EXPLORER}/txns/${hash}`;
export const accountUrl = (account: string) => `${EXPLORER}/address/${account}`;
export const attestationUrl = (requestId: string) => `${OUTLAYER}/attestations/by-request/${requestId}`;
export const wasmUrl = (sha256: string) => (WASM_STORE ? `${WASM_STORE}/${sha256}.wasm` : null);

/** Whole base units in one lot, e.g. 1 for a 1 tNEAR lot. */
export const LOT_UNITS = Number(MARKET.lot) / 10 ** MARKET.baseDecimals;

/** One lot, labelled for people: "1 tNEAR". */
export const LOT_LABEL = `${LOT_UNITS} ${MARKET.baseSymbol}`;

/** Lots one ticket can carry. The matcher crosses at most this many per side per batch. */
export const MAX_LOTS = 10;
