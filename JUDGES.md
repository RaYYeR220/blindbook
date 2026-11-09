# Reviewing Blindbook in five minutes

Each step says what to do and what you should see.

## 1. See a block cross (1 minute, no wallet)

Open [blindbook-phi.vercel.app/app](https://blindbook-phi.vercel.app/app) and press **Sell 1 lot in the dark**.

**You should see:** after about 15 seconds, a batch print with the midpoint, your lot filled at the midpoint minus at most 5 bps, a settlement transaction link, and the batch's verification checks turning green.

## 2. Check a batch yourself (2 minutes, terminal)

```bash
pnpm install && pnpm --filter @blindbook/sdk build && pnpm --filter @blindbook/client build
pnpm verify:batch 9 --tx 8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa --sender blindbook-crank-e4c061.testnet
```

**You should see:** seven `PASS` lines:

1. result signature accepted on-chain;
2. both legs settled in `execute_intents`;
3. the price came from two signed 1Click quotes;
4. there is an Intel TDX attestation for the run;
5. the enclave ran the pinned build (`a352985a…a980`);
6. the enclave input is the exact batch the pool sent;
7. the venue key is registered on the verifier.

You can also read each artefact directly:

| What | Where |
| --- | --- |
| The whole batch in one transaction: request → enclave run → signed result → `intents_executed` → `batch_settled` | [`8Qa2X8bu…`](https://testnet.nearblocks.io/txns/8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa) |
| TDX quote, executed wasm hash, input and output hashes | [OutLayer attestation 15030](https://testnet-api.outlayer.ai/attestations/by-request/15030) |
| The pool's record of the batch | `get_batch({"id":"9"})` on `pool.blindbook-dev.testnet` |
| Pinned matcher build, result key, venue key | `get_config({})` on `pool.blindbook-dev.testnet` |

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
