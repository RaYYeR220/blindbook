# Blindbook protocol

Blindbook is a block-crossing venue for NEAR Intents. Orders are sealed to a matcher that runs inside an Intel TDX enclave (OutLayer), crossed at a verified midpoint in periodic batches, and settled atomically by the NEAR Intents verifier. No server holds orders, keys or funds.

```
trader ──seal──▶ pool contract (NEAR) ──run_batch──▶ OutLayer TEE: matcher
                     ▲                                   │ open, validate, cross, plan
                     └──────── signed result ◀───────────┘
                     │
                     └──execute_intents (one call per sub-batch)──▶ intents.near
```

## 1. Orders

### 1.1 Lots

A market has a fixed lot size in base units. An order is a bundle of **lots**, and each lot is one signed verifier payload holding exactly one `token_diff`:

| side | diff |
| --- | --- |
| sell | `{ base: "-<lot>", quote: "<limit_quote>" }` |
| buy | `{ base: "<lot>", quote: "-<limit_quote>" }` |

`limit_quote` is the trader's limit for that lot: the minimum quote received on a sell, the maximum paid on a buy. Because the verifier applies the signed amounts exactly, **no lot can ever settle at a worse price than its limit**, whatever the matcher does.

Each lot payload uses the verifier's versioned nonce:

```
5628f6c6 | 00 | salt (4) | nonce deadline, i64 ns LE (8) | random (15)
```

The salt is the verifier's `current_salt`. The intent `deadline` must be no later than the nonce deadline.

Supported signing standards:
- `nep413`, for NEAR accounts. A named account must first register the key on the verifier with `add_public_key`; implicit accounts work as-is.
- `erc191`, for EVM wallets. `signer_id` is the lowercase `0x` address, and the recovery id is `0` or `1`.

### 1.2 Order body

The plaintext sealed inside the envelope:

```json
{
  "v": 1,
  "market": "wnear-usdc",
  "owner": "alice.near",
  "side": "sell",
  "min_lots": 1,
  "lots": [
    { "index": 0, "limit_quote": "354000", "intent": { "standard": "nep413", "payload": { "...": "..." }, "public_key": "ed25519:...", "signature": "ed25519:..." } }
  ]
}
```

`owner` is the verifier account that signed every lot. An order's deadline is the earliest deadline among its lots.

### 1.3 Envelope

Orders are sealed to the matcher's X25519 key, which `pool.get_matcher_key()` publishes. The layout:

```
version 0x01 | ephemeral X25519 public key (32) | XChaCha20 nonce (24) | AEAD ciphertext
key   = HKDF-SHA256(ikm = X25519(ephemeral, matcher), salt = eph_pub || matcher_pub, info = "blindbook/v1/order")
aad   = version || eph_pub
plain = u32 LE length || body || zero padding to a bucket of 2, 4, 8, 16, 32 or 64 KiB
```

`blindbook-core::envelope` (Rust) and `@blindbook/sdk` (TypeScript) implement this byte for byte. A shared test vector proves it.

### 1.4 Cancelling

- **Before the matcher ingests an order:** `pool.cancel_order(id)` refunds its storage deposit.
- **After ingestion:** the trader signs a *cancel ticket* for each remaining lot: the same signer and nonce with `"intents": []`. They submit it to the verifier directly, and the nonce is consumed. The matcher drops any lot whose nonce has been used.

This needs no permission from the venue.

## 2. Pool contract

| Method | Who | What |
| --- | --- | --- |
| `submit_order(market, sealed_b64)` | anyone, payable | Stores the sealed order. The attached deposit covers its storage and is refunded once the matcher ingests or rejects the order. Emits `order_submitted`. |
| `cancel_order(id)` | the submitter | Removes a pending (not yet ingested) order and refunds the deposit. |
| `run_batch()` | anyone, payable | Sends the pending orders to the matcher through `outlayer.request_execution` (yield/resume). Unused OutLayer deposit is refunded to the caller. |
| `on_result(batch_id, order_ids)` | private | Verifies the matcher's signature over the result, refunds storage for ingested and rejected orders, records the batch, then calls `execute_intents` once per sub-batch. |
| `on_settled(batch_id)` | private | Records which sub-batches settled and emits `batch_settled`. |
| views | — | `get_config`, `get_markets`, `get_matcher_key`, `get_pending`, `get_batch`, `get_batches`, `get_stats` |

The matcher runs as an OutLayer **project**, pinned to one wasm hash (`version_key`). Its keys are bound to the pool contract as the calling predecessor, so nobody else can obtain them by running the same code.

## 3. Matcher I/O

### 3.1 Input (`input_data`, public)

```json
{
  "v": 1,
  "op": "batch",
  "network": "mainnet",
  "verifier": "intents.near",
  "now_ns": "1794225600000000000",
  "markets": [
    { "id": "wnear-usdc", "base": "nep141:wrap.near", "quote": "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1",
      "lot": "100000000000000000000000",
      "reference": { "base": "nep141:wrap.near", "quote": "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1", "probe_base": "10000000000000000000000000", "probe_quote": "36000000" } }
  ],
  "params": { "fee_bps": 5, "max_spread_bps": 150, "max_lots_per_side": 6, "max_sub_batches": 4, "deadline_margin_s": 90 },
  "orders": [ { "id": "17", "seq": 17, "market": "wnear-usdc", "sealed": "<base64>" } ]
}
```

- `op: "keys"` asks the matcher to publish its public keys and ignores `orders`.
- `reference` names the assets used to fetch 1Click quotes. On testnet, the test tokens map to their mainnet equivalents.
- `probe_*` are the quote sizes for the bid (sell `probe_base`) and the ask (buy with `probe_quote`).

### 3.2 Output (callback return value, public)

```json
{ "result": "<canonical JSON string>", "sig": "ed25519:<base58>", "key": "ed25519:<base58>" }
```

`sig` is an ed25519 signature by the matcher's result key over `sha256(result)`. The pool checks it against the pinned key with `env::ed25519_verify` before acting on anything.

`result` decodes to:

```json
{
  "v": 1,
  "op": "batch",
  "ingested": ["17"],
  "rejected": [ { "id": "18", "reason": "invalid" } ],
  "batches": [
    { "market": "wnear-usdc", "mid": { "quote": "...", "base": "..." }, "lot_mid_quote": "...", "matched_lots": 2,
      "base_volume": "...", "quote_volume": "...", "rebated": "...", "venue_fee": "...", "quotes": ["<1click digest>", "<1click digest>"],
      "skipped": null }
  ],
  "settle": [ [ { "standard": "nep413", "...": "..." } ] ],
  "book": { "orders": 3, "lots": 9 }
}
```

- `settle` holds one array of signed payloads per sub-batch, in submission order: sells, then buys, then the venue payload. Each sub-batch nets to zero under the verifier's fee on its own, and the matcher simulates each one against the live verifier before it is included.
- `book` reports aggregate counts only.
- For `op: "keys"`, `result` is:

```json
{ "v": 1, "op": "keys", "matcher_x25519": "<hex>", "venue": "ed25519:<base58>", "result_key": "ed25519:<base58>" }
```

### 3.3 Privacy

Everything in `input_data` and in the result is public on chain:
- The input carries only ciphertext and market configuration.
- The result reveals only what settlement reveals anyway: crossed lots, volumes and the reference price.
- Resting orders, their sizes and their limits never leave the enclave.

## 4. Venue

The venue is a verifier account whose only signing key is held by the enclave. In each sub-batch it signs:
1. a `token_diff` closure that balances the sub-batch under the verifier's protocol fee, and
2. one `transfer` per trader, returning their price improvement minus the venue fee.

The venue holds a small balance of each token to cover fee dust.
