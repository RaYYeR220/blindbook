# Reviewing Blindbook in five minutes

Each step says what to do and what you should see.

## 1. See a block cross (1 minute, no wallet)

Open [blindbook-phi.vercel.app/app](https://blindbook-phi.vercel.app/app) and press **Sell 1 lot in the dark**. The free demo runs on testnet.

**You should see:** after about 25 seconds, a batch print with the midpoint, your lot filled at the midpoint minus at most 5 bps, a settlement transaction link, and the batch's verification checks turning green.

## 2. Check a batch yourself (2 minutes, terminal)

The same code runs on mainnet. Check its first batch, then a testnet one.

```bash
pnpm install && pnpm --filter @blindbook/sdk build && pnpm --filter @blindbook/client build

# 1. mainnet batch 2: one 0.1 NEAR lot a side, settled on intents.near
pnpm verify:batch 2 --tx 5HgpGzUbGPENVWwcf5kQNgWX2hMCTJSMqrjhGky4UFiZ --sender demo.blindbook.near --mainnet

# 2. testnet batch 9: two lots a side
pnpm verify:batch 9 --tx 8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa --sender blindbook-crank-e4c061.testnet
```

**You should see:** seven `PASS` lines for each batch:

1. result signature accepted on-chain;
2. both legs settled in `execute_intents`;
3. the price came from two signed 1Click quotes;
4. there is an Intel TDX attestation for the run;
5. the enclave ran the pinned build (mainnet `435b7863…7bc9`, testnet `a352985a…a980`);
6. the enclave input is the exact batch the pool sent;
7. the venue key is registered on the verifier.

In mainnet batch 2 the seller signed a limit of 3.71828 USDC per NEAR and got 3.73067. The buyer signed 3.75565 and paid 3.73439. The lot mid was 3.73253.

You can also read each artefact directly:

| What | Mainnet batch 2 | Testnet batch 9 |
| --- | --- | --- |
| The whole batch in one transaction: request → enclave run → signed result → `intents_executed` → `batch_settled` | [`5HgpGzUb…`](https://nearblocks.io/txns/5HgpGzUbGPENVWwcf5kQNgWX2hMCTJSMqrjhGky4UFiZ) | [`8Qa2X8bu…`](https://testnet.nearblocks.io/txns/8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa) |
| TDX quote, executed wasm hash, input and output hashes | [OutLayer attestation 2983](https://api.outlayer.ai/attestations/by-request/2983) | [OutLayer attestation 15030](https://testnet-api.outlayer.ai/attestations/by-request/15030) |
| The pool's record of the batch | `get_batch({"id":"2"})` on `pool.blindbook.near` | `get_batch({"id":"9"})` on `pool.blindbook-dev.testnet` |
| Pinned matcher build, result key, venue key | `get_config({})` on `pool.blindbook.near` | `get_config({})` on `pool.blindbook-dev.testnet` |
| The run record | [`results/e2e-mainnet-2026-11-11T14-37-10-533Z.json`](results/e2e-mainnet-2026-11-11T14-37-10-533Z.json) | [`results/e2e-testnet-2026-11-09T19-59-31-034Z.json`](results/e2e-testnet-2026-11-09T19-59-31-034Z.json) |

Every mainnet and testnet account is listed under [Deployments](README.md#deployments).

## 3. Check the headline number (1 minute)

```bash
pnpm baseline
```

**You should see:** the public route's cost against the midpoint for blocks from 10 to 500,000 NEAR, measured live with 1Click dry quotes. Large sells end in `No liquidity available`. The README's numbers come from [`results/`](results), rendered by `pnpm readme`.

## 4. Read the guarantees in code (1 minute)

| Claim | Code |
| --- | --- |
| A lot can never fill worse than its signed limit | The verifier applies the signed `token_diff` exactly. Lots are checked in [`crates/core/src/intent.rs`](crates/core/src/intent.rs), and the property is fuzzed in [`crates/core/tests/invariants.rs`](crates/core/tests/invariants.rs) |
| The price comes only from a signed 1Click quote | [`crates/core/src/quote.rs`](crates/core/src/quote.rs) (byte-compatible with the official SDK; tamper tests included) |
| Nothing settles unless the enclave's signature verifies | `on_result` in [`contracts/pool/src/lib.rs`](contracts/pool/src/lib.rs) (`env::ed25519_verify` against the pinned key) |
| The batch nets to zero under the verifier's fee | [`crates/core/src/settle.rs`](crates/core/src/settle.rs) (matches a settled testnet batch to the unit) |
| Resting orders never leave the enclave | [`matcher/src/lib.rs`](matcher/src/lib.rs) and the `results_reveal_nothing_about_resting_orders` test |
| Anyone can cancel without the venue | [`packages/sdk/src/order.ts`](packages/sdk/src/order.ts) `cancelTickets` |

## 5. Run the tests

```bash
cargo test -p blindbook-core -p blindbook-matcher
(cd contracts/pool && AWS_LC_SYS_PREBUILT_NASM=1 cargo test)
pnpm -r test
```

The limits we know about are listed under [Honest limits](README.md#honest-limits). The status of every public claim is in [CLAIMS.md](CLAIMS.md).
