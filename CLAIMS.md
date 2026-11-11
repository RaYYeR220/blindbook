# Claims and their evidence

Every public statement about Blindbook carries a tier:

- **REPRODUCIBLE**: run the command and get the same result.
- **VERIFIED-LIVE**: a public on-chain or attested artefact you can open.
- **MODELED**: computed from measured inputs, but not observed directly.
- **NOT-CLAIMED**: we deliberately do not claim it.

| # | Claim | Tier | Evidence |
| --- | --- | --- | --- |
| 1 | Selling 150,000 NEAR on the public NEAR Intents route cost about 102 bps against the midpoint on Nov 9, 2026, and 200,000 NEAR or more got `No liquidity available` | REPRODUCIBLE | `pnpm baseline`. The Nov 9 run is [`results/baseline-2026-11-09.json`](results/baseline-2026-11-09.json). The numbers move with the market. |
| 2 | Two opposite sealed orders crossed at the midpoint and both legs settled in one transaction | VERIFIED-LIVE | Testnet batch 9, tx [`8Qa2X8bu…`](https://testnet.nearblocks.io/txns/8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa). Reproduce with `pnpm e2e:testnet`. |
| 3 | In batch 9 the seller (limit 3.5977) received 3.6132 and the buyer (limit 3.6339) paid 3.6168, around a midpoint of 3.61498 | VERIFIED-LIVE | `get_batch({"id":"9"})` on `pool.blindbook-dev.testnet`, plus [`results/e2e-testnet-2026-11-09T19-59-31-034Z.json`](results) |
| 4 | The matcher ran inside Intel TDX, ran the exact pinned build, and received exactly the input the pool sent | VERIFIED-LIVE | [OutLayer attestation 15030](https://testnet-api.outlayer.ai/attestations/by-request/15030). Checked by `pnpm verify:batch 9 …` |
| 5 | The pool settles nothing unless the matcher's result signature verifies against the pinned key | REPRODUCIBLE | `contracts/pool` unit tests for forged and tampered results. On testnet, rejected forgeries are recorded as `rejected_signature` batches. |
| 6 | No lot can settle at a worse price than the limit its owner signed, even if the enclave is compromised | REPRODUCIBLE | The verifier enforces signed `token_diff` amounts. Lot shape is checked in `crates/core/src/intent.rs`. The engine fuzz in `crates/core/tests/invariants.rs` passes 5,000 random books with 0 violations. |
| 7 | The reference price comes only from 1Click quotes whose ed25519 signatures verify | REPRODUCIBLE | `crates/core/src/quote.rs` with live-quote fixtures and tamper tests. Each batch record lists the quote digests the enclave verified. |
| 8 | Ciphertext length does not reveal an order's lot count | REPRODUCIBLE | `crates/core/src/envelope.rs` bucket padding tests, and the shared Rust/TypeScript vector |
| 9 | Settlement batches are fee-exact under the verifier's protocol fee | REPRODUCIBLE | `crates/core/src/settle.rs` reproduces a testnet settlement to the unit, and fuzzes 10,000 batches at 5 fee levels |
| 10 | Anyone can cancel their open lots without the venue | REPRODUCIBLE | Cancel tickets in `packages/sdk/src/order.ts`. The matcher drops lots whose nonce is consumed (`matcher/tests/reconcile.rs`). |
| 11 | Blindbook costs a block at most 5 bps against the midpoint when the other side is in the batch | MODELED | The venue fee is capped per side and taken only from price improvement (`crates/core/src/engine.rs`). Whether a block finds the other side depends on liquidity. |
| 15 | The private broker's answers come from an attested NEAR AI model: the TDX quote binds a fresh nonce and the signing address, NVIDIA's GPU attestation passes, and the answer is signed by that address | VERIFIED-LIVE | `POST /api/broker` on [blindbook-phi.vercel.app](https://blindbook-phi.vercel.app/app) returns the plan with its three checks (`apps/web/src/lib/server/broker.ts`). The Intel signature chain of the quote is not re-verified. |
| 12 | A block of any size finds a counterparty | NOT-CLAIMED | Early liquidity is a labelled house account; outside flow is not guaranteed |
| 13 | Trades stay private after settlement | NOT-CLAIMED | Fills are public on `intents.near` once they settle |
| 14 | Mainnet volume | NOT-CLAIMED | Not claimed until the mainnet deployment is live and linked here |

## What is simulated

- **On testnet**, the base and quote tokens are test tokens: `tnear-bbcced2c.testnet` and `tusdc-bbcced2c.testnet`. They settle on a deployment of the same verifier bytecode that runs as `intents.near` (defuse 0.4.5). The reference price is the live **mainnet** wNEAR/USDC midpoint from 1Click.
- **The house account** is ours, and it is the counterparty in the public demo.
- **The matcher's unit and integration tests** run against a mock verifier that enforces signatures, nonces, salts, balances and the fee invariant. The testnet runs above use the real verifier.
