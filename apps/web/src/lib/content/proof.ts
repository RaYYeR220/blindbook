/**
 * Every public identifier the landing page shows or links to. Moving the page
 * to mainnet means editing this file and nothing else.
 *
 * Numbers that come out of a measurement (route costs, fill prices) are not
 * here: they are read from the files in the repo's `results/` folder.
 */

export const NETWORK = {
  id: "testnet",
  label: "NEAR testnet",
  rpc: "https://test.rpc.fastnear.com",
  pool: "pool.blindbook-dev.testnet",
  explorer: "https://testnet.nearblocks.io",
  outlayerApi: "https://testnet-api.outlayer.ai",
  wasmStore: "https://test.fastfs.io/blindbook-dev.testnet/outlayer.testnet",
  /** Decimals of the base (NEAR) and quote (USDC) tokens. */
  baseDecimals: 24,
  quoteDecimals: 6,
} as const;

export const txUrl = (hash: string) => `${NETWORK.explorer}/txns/${hash}`;
export const accountUrl = (account: string) => `${NETWORK.explorer}/address/${account}`;

const WASM_SHA256 = "a352985a6807d716158c8aed651af0882aa9da79542911225df129dead02a980";

export const MATCHER = {
  wasmSha256: WASM_SHA256,
  wasmUrl: `${NETWORK.wasmStore}/${WASM_SHA256}.wasm`,
} as const;

/**
 * The batch the page walks through. The run itself (fills, txs, limits) is read
 * from the `results/e2e-<network>-*.json` file whose `run_batch` tx matches
 * `crankTx`. The rest is the pool's own record: `get_batch {"id":"9"}`.
 */
export const BATCH = {
  id: 9,
  crankTx: "8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa",
  outlayerRequest: 15030,
  quoteDigests: ["81XZUCet19pngDZCaAfzkAUXpSYjB4Xibz6DGLHHGqu6", "3wvV2Avez7UpEex3WQtaVJFQu17z6UopaCVRrGzJ2BAD"],
  resultHash: "1a62964ae7216dfc7edf1bb5d9998dfd7ebe0893d4490cbe828dc8482737bd14",
  /** `BAND_BPS` in scripts/e2e-testnet.ts: each limit sits this far from the reference mid. */
  limitBandBps: 50,
} as const;

export const attestationUrl = (request: number) => `${NETWORK.outlayerApi}/attestations/by-request/${request}`;

/** Route-cost baseline in `results/`. */
export const BASELINE_FILE = "baseline-2026-11-09.json";

export const TESTS = { core: 60, matcher: 94, pool: 46, sdk: 11 } as const;
export const FUZZ = { books: 5000, violations: 0 } as const;

/** Earlier settlement runs on testnet, shown in the hero marquee after the batch itself. */
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
