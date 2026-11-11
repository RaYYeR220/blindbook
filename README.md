# Blindbook

**Sell size without being seen.** Blindbook is a block-crossing venue, a dark pool, for NEAR Intents. You seal a block at your own limit. Each batch, a matcher running inside an Intel TDX enclave crosses it with the other side at a midpoint signed by 1Click, and both legs settle in one `execute_intents` call on the NEAR Intents verifier. Nobody sees a resting order: not other traders, not solvers, not us.

**Live:** [blindbook-phi.vercel.app](https://blindbook-phi.vercel.app) · try a cross with no wallet at [/app](https://blindbook-phi.vercel.app/app) · re-check any batch with `pnpm verify:batch`.

<!-- numbers:start -->
On the public NEAR Intents route, selling **150,000 NEAR (~$532,000)** cost **101.8 bps (about $5,400)** against the midpoint, and from **200,000 NEAR** the solvers answered `No liquidity available`. Blindbook crosses the same block at the midpoint for a venue fee of at most 5 bps, taken only out of price improvement. *(1Click dry quotes, measured 2026-11-09, [`results/baseline-2026-11-09.json`](results/baseline-2026-11-09.json).)*
<!-- numbers:end -->

## Why

Big holders have two choices today. They can sell on the public route and pay for the impact, or stop at the size where solvers stop quoting. Or they can hand the whole order to an OTC desk that then knows their hand. DAOs selling runway, funds rebalancing and treasury agents all leak their intent the moment they ask for a price.

Blindbook gives a block somewhere to rest where it can't be read, finds the other side, and settles both legs atomically across the chains NEAR Intents supports.

## How a block crosses

1. **Seal.** Your wallet signs the order as N lots. Each lot is one `token_diff` at your own limit. The lots are encrypted to the matcher's X25519 key, which exists only inside the enclave, and padded to a fixed size. The ciphertext is stored in the `pool` contract on NEAR.
2. **Batch.** Anyone can call `pool.run_batch()`. The pool sends the new ciphertexts to the matcher, an [OutLayer](https://outlayer.ai) project running in Intel TDX.
3. **Cross at the mid.** Inside the enclave, the matcher opens the orders and validates every signature against the verifier. It fetches a bid and an ask from 1Click and accepts them only if their ed25519 signatures verify. It then crosses compatible lots at the midpoint, one uniform price for everyone in the batch. Resting orders stay in enclave storage.
4. **Settle.** The enclave returns signed settlement bundles. The pool checks the signature against the pinned key on-chain, then calls `execute_intents`:
   - Sells, buys and the venue leg settle in one atomic call.
   - The venue leg balances the verifier's fee and returns each trader's price improvement.
   - Proceeds can be withdrawn to any chain NEAR Intents supports.

**Worst case: your own limit.** The verifier applies the exact amounts each trader signed, so even a compromised enclave can't fill a lot worse than its limit. The most it could do is leak resting orders.

```
trader ──seal──▶ pool (NEAR) ──run_batch──▶ OutLayer · Intel TDX · matcher
                     ▲                         │ open · validate · price · cross
                     └──── signed result ◀─────┘
                     └──── execute_intents ───▶ intents.near ──▶ 39 chains
```

## Try it

<!-- try:start -->
**Hosted demo: [blindbook-phi.vercel.app/app](https://blindbook-phi.vercel.app/app).** Press **Sell 1 lot in the dark** or **Buy 1 lot in the dark**. A server-held demo account seals one lot, a house account rests the other side, and the batch cranks. About 25 seconds later you see the batch print, the settlement transaction, and both legs filled at the midpoint within their limits. No wallet, no funds, no install.

**From a terminal, on testnet:**

```bash
pnpm install
pnpm --filter @blindbook/sdk build && pnpm --filter @blindbook/client build
pnpm baseline        # measure the public route's cost curve now -> results/baseline-<date>.json
pnpm e2e:testnet     # seal two opposite orders, crank one batch, check both legs settled at the mid
pnpm verify:batch 9 --tx 8Qa2X8buwY86ztwXvrSk6rBNRNV2hReQzEHe82MPgMUa --sender blindbook-crank-e4c061.testnet
```

`pnpm verify:batch` prints seven checks, and every one should read `PASS`:

1. the matcher's result signature was accepted on-chain;
2. both legs settled in `execute_intents`;
3. the price came from signed 1Click quotes;
4. there is an Intel TDX attestation for the run;
5. the enclave ran the pinned matcher build;
6. it ran on exactly the input the pool sent;
7. the venue key is registered on the verifier.

`pnpm e2e:testnet` needs testnet keys in `~/.near-credentials/testnet/` for the accounts it uses (see the script header).
<!-- try:end -->

**As an agent (MCP).** Add the server to any MCP client:

```json
{
  "mcpServers": {
    "blindbook": {
      "command": "node",
      "args": ["<path-to-repo>/packages/mcp/dist/index.js"],
      "env": { "BLINDBOOK_NETWORK": "testnet", "BLINDBOOK_SECRET_KEY": "ed25519:...", "BLINDBOOK_MAX_LOTS": "10" }
    }
  }
}
```

| Tool | What it does | Needs a key |
| --- | --- | --- |
| `blindbook_markets` | Markets, lot sizes, pool and verifier | no |
| `blindbook_route_cost` | What a block would cost on the public route right now | no |
| `blindbook_batches` | Recent batches: lots crossed, volumes, the midpoint, settlement | no |
| `blindbook_balance` | The agent's verifier balances | yes |
| `blindbook_place_order` | Seal and submit a block at a limit, in lots | yes |
| `blindbook_order_status` | Lots still open versus consumed | yes |
| `blindbook_cancel_order` | Cancel open lots with cancel tickets on the verifier | yes |

The key never leaves the MCP process. `BLINDBOOK_MAX_LOTS` is a hard cap per order.

## What runs where

| Part | Where | Code |
| --- | --- | --- |
| Crossing engine: midpoint, lots, time priority, min fill, self-trade rule, fee closure | Rust library, in the enclave | [`crates/core`](crates/core) |
| Matcher: opens sealed orders, validates lots, prices, crosses, plans settlement, signs | OutLayer project, Intel TDX, `wasm32-wasip2` | [`matcher`](matcher) |
| Pool: sealed order store, permissionless crank, on-chain result verification, settlement | NEAR contract | [`contracts/pool`](contracts/pool) |
| SDK: build, sign (NEP-413 / ERC-191) and seal orders; cancel tickets; route cost | TypeScript | [`packages/sdk`](packages/sdk) |
| Client: place, track, cancel, crank, verify | Node | [`packages/client`](packages/client) |
| MCP server for agents | Node | [`packages/mcp`](packages/mcp) |
| House liquidity and keeper | Node | [`agents/house`](agents/house) |
| Web app and public demo | Next.js | [`apps/web`](apps/web) |

The full message formats are in [`docs/PROTOCOL.md`](docs/PROTOCOL.md).

## NEAR stack used

- **NEAR Intents verifier** (`intents.near`): signed `token_diff` lots, the venue's fee-exact closure, atomic `execute_intents`, cancel tickets.
- **1Click API**: signed dry quotes as the reference price; the public route Blindbook is measured against.
- **OutLayer**: verifiable off-chain execution in Intel TDX, called from the pool via yield/resume. Its project-bound keys hold the order-decryption key and the venue's signing key, and nobody else can obtain them.
- **NEAR accounts**: NEP-413 signatures from NEAR wallets. EVM wallets sign ERC-191 as implicit `0x` accounts.
- **NEAR contract** (`near-sdk` 5.30) with `env::ed25519_verify` checking the enclave's result before anything settles.
- **NEAR AI Cloud**: the private broker in `/app` turns a plain-language mandate ("sell 3 lots over the next hour, never below 3.40") into a lot plan with GLM 5.3 Flash running in a TEE. Before each answer it attests the model fresh: the TDX quote must carry our nonce and the signing address, NVIDIA's GPU attestation must pass, and the answer must be signed by that address. The plan is only a suggestion; the trader still signs every lot.

## Tests

| Suite | Count |
| --- | --- |
| `crates/core`: engine, price, quotes, envelope, lot checks, settlement planning | 60, including a fuzz test over 5,000 random books (0 invariant violations) |
| `matcher`: full batches against a mock verifier that enforces signatures, nonces, salts, balances and the fee invariant | 94 |
| `contracts/pool` | 46 (95.6% line coverage) |
| `packages/sdk`: including byte-for-byte envelope compatibility with Rust and SDK-signed lots passing the Rust validator | 11 |

```bash
cargo test -p blindbook-core -p blindbook-matcher
(cd contracts/pool && AWS_LC_SYS_PREBUILT_NASM=1 cargo test)   # the env var is needed on Windows only
pnpm -r test
```

## Honest limits

- **Settlement is public.** After a cross, the fills are visible on `intents.near` like any other intent. Before the cross, nothing is.
- **Early liquidity is ours.** Until outside flow arrives, a house account (`agents/house`) quotes both sides around the midpoint. It is labelled as house liquidity and never trades with itself.
- **You trust the enclave and its operator.** Blindbook relies on Intel TDX and on OutLayer's worker set:
  - A compromised enclave could reveal resting orders.
  - It still could not fill anyone worse than their signed limit.
  - It could not move funds the traders did not sign.
  - The pool verifies every result signature on-chain. The TDX attestation for each run is public.
- **Small lots on testnet today.** The full path runs on NEAR testnet against a deployment of the same verifier code as `intents.near`, with a mainnet reference price. The mainnet deployment uses 0.1 NEAR lots.
- **One signature per lot from wallets.** NEAR and EVM wallets sign each lot separately today. Agents and scripts sign locally.

## Also by us at NEAR HACKATHON

[CHAIR](https://github.com/RaYYeR220/chair), a memecoin that runs its own central bank: a TEE model rules on its rates every day, and the token contract verifies the signature itself and clamps the ruling to a constitution fixed at launch.

## License

MIT
