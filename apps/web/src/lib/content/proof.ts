/**
 * Every public identifier the landing page shows or links to.
 *
 * Numbers that come out of a measurement (route costs, fill prices) are not
 * here: they are read at build time from the files in the repo's `results/`
 * folder. Nothing on the landing page reads mainnet live.
 */

export interface Deployment {
  id: "mainnet" | "testnet";
  label: string;
  rpc: string;
  pool: string;
  verifier: string;
  explorer: string;
  outlayerApi: string;
  /** The pinned matcher build, and where its wasm is published. */
  matcher: { wasmSha256: string; wasmUrl: string };
  /** Decimals of the base (NEAR) and quote (USDC) tokens. */
  baseDecimals: number;
  quoteDecimals: number;
}

const MAINNET_WASM = "435b786312128aefc2a0a60ffa4d0232a22cfcf0ad31e492075519b909467bc9";
const TESTNET_WASM = "a352985a6807d716158c8aed651af0882aa9da79542911225df129dead02a980";

export const MAINNET: Deployment = {
  id: "mainnet",
  label: "NEAR mainnet",
  rpc: "https://rpc.mainnet.fastnear.com",
  pool: "pool.blindbook.near",
  verifier: "intents.near",
  explorer: "https://nearblocks.io",
  outlayerApi: "https://api.outlayer.ai",
  matcher: {
    wasmSha256: MAINNET_WASM,
    wasmUrl: `https://main.fastfs.io/blindbook.near/outlayer.near/${MAINNET_WASM}.wasm`,
  },
  baseDecimals: 24,
  quoteDecimals: 6,
};

export const TESTNET: Deployment = {
  id: "testnet",
  label: "NEAR testnet",
  rpc: "https://test.rpc.fastnear.com",
  pool: "pool.blindbook-dev.testnet",
  verifier: "intents-bbcced2c.testnet",
  explorer: "https://testnet.nearblocks.io",
  outlayerApi: "https://testnet-api.outlayer.ai",
  matcher: {
    wasmSha256: TESTNET_WASM,
    wasmUrl: `https://test.fastfs.io/blindbook-dev.testnet/outlayer.testnet/${TESTNET_WASM}.wasm`,
  },
  baseDecimals: 24,
  quoteDecimals: 6,
};

/**
 * The network the hosted demo trades on and the live pool counter reads. It is
 * fixed at build time, and the free demo stays on testnet.
 */
export const NETWORK = TESTNET;

export const txUrl = (hash: string, net: Deployment = NETWORK) => `${net.explorer}/txns/${hash}`;
export const accountUrl = (account: string, net: Deployment = NETWORK) => `${net.explorer}/address/${account}`;
export const attestationUrl = (request: number, net: Deployment = NETWORK) =>
  `${net.outlayerApi}/attestations/by-request/${request}`;

/**
 * A settled batch the page walks through. The run itself (fills, txs, the mid)
 * is read from the `results/e2e-<network>-*.json` file whose `run_batch` tx
 * matches `crankTx`. The rest is the pool's own record: `get_batch {"id":"<id>"}`.
 */
export interface PinnedBatch {
  net: Deployment;
  id: number;
  crankTx: string;
  /** The account that cranked the batch, for `pnpm verify:batch --sender`. */
  cranker: string;
  outlayerRequest: number;
  /** For run records that predate the pool's batch record in the file. */
  quoteDigests?: readonly string[];
  resultHash?: string;
  /**
   * The limits each side signed, in USDC per NEAR, read from the signed
   * `token_diff`s in the crank tx. Without them the page derives the limits from
   * the run's reference mid and `limitBandBps`.
   */
  signedLimits?: { sell: number; buy: number };
  /** `BAND_BPS` in scripts/e2e-testnet.ts: each limit sits this far from the reference mid. */
  limitBandBps: number;
}

/** The first mainnet batch. Run record: results/e2e-mainnet-2026-11-11T14-37-10-533Z.json */
export const MAINNET_BATCH: PinnedBatch = {
  net: MAINNET,
  id: 2,
  crankTx: "5HgpGzUbGPENVWwcf5kQNgWX2hMCTJSMqrjhGky4UFiZ",
  cranker: "demo.blindbook.near",
  outlayerRequest: 2983,
  signedLimits: { sell: 3.71828, buy: 3.75565 },
  limitBandBps: 50,
};

/** Testnet batch 9. Run record: results/e2e-testnet-2026-11-09T19-59-31-034Z.json */
export const TESTNET_BATCH: PinnedBatch = {
  net: TESTNET,
  id: 9,
  crankTx: "8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa",
  cranker: "blindbook-crank-e4c061.testnet",
  outlayerRequest: 15030,
  quoteDigests: ["81XZUCet19pngDZCaAfzkAUXpSYjB4Xibz6DGLHHGqu6", "3wvV2Avez7UpEex3WQtaVJFQu17z6UopaCVRrGzJ2BAD"],
  resultHash: "1a62964ae7216dfc7edf1bb5d9998dfd7ebe0893d4490cbe828dc8482737bd14",
  limitBandBps: 50,
};

/** The command that re-checks a batch from public sources. */
export const verifyCommand = (b: PinnedBatch) =>
  `pnpm verify:batch ${b.id} --tx ${b.crankTx} --sender ${b.cranker}${b.net.id === "mainnet" ? " --mainnet" : ""}`;

/** Route-cost baseline in `results/`. */
export const BASELINE_FILE = "baseline-2026-11-09.json";

export const TESTS = { core: 60, matcher: 94, pool: 46, sdk: 11 } as const;
export const FUZZ = { books: 5000, violations: 0 } as const;

/** Earlier settlement runs on testnet, shown in the hero marquee after the pinned batches. */
export const SETTLEMENT_RUNS = [
  { tx: "6jfXAVpxhABkusA7McxE79Qap7A4MPMYByhJpPyMfBxz", note: "both legs · 1 tx · NEAR testnet" },
  { tx: "8f4ubY6LPvmNXDuBbNoBgr6sQcTmji2jn12av4qCVqAZ", note: "both legs · 1 tx · fee from improvement only" },
  { tx: "DZy7hnKUinmtCFxXapz3854mHifdHG1mHZELZoTe3Xi5", note: "settle_many · 3 × 13 = 39 intents · 1 call" },
] as const;

export const LINKS = {
  app: "/app",
  protocol: "/protocol",
  cancelTicket: "/protocol#14-cancelling",
  github: "https://github.com/RaYYeR220/blindbook",
  x: "#",
} as const;
