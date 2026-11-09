// Testnet end-to-end for the pool contract, against the `pool-echo` OutLayer module.
//
//   tsx e2e.ts keys     run_keys from the cranker (prints status, reported keys)
//   tsx e2e.ts pin      owner pins the result key the last keys run reported
//   tsx e2e.ts settle   fresh seller/buyer/venue payloads -> submit_order -> run_batch
//                       -> OutLayer -> on_result -> execute_intents -> on_settled
//   tsx e2e.ts forge [wrong_key|tamper]   a result the pool must reject; then cancel_order
//   tsx e2e.ts crank    one run_batch from the cranker, no new orders
//   tsx e2e.ts real [seller,buyer] [sellLimit] [buyLimit]
//                       against the real matcher project: two sealed orders placed with
//                       @blindbook/client (packages/client must be built), then run_batch
//
// keys/pin/settle/forge expect the pool to point at the pool-echo project
// (init-args.testnet.json); `real` expects blindbook-dev.testnet/blindbook-matcher.
//
// Keys come from ~/.near-credentials (testnet keychain) and the defuse spike's
// key file; nothing secret is printed or written. Evidence goes to ./evidence/.
import { Account, JsonRpcProvider, KeyPairSigner, actions } from "near-api-js";
import { ed25519 } from "@noble/curves/ed25519.js";
import { sha256 } from "@noble/hashes/sha2.js";
import bs58 from "bs58";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const RPC = process.env.NEAR_RPC ?? "https://test.rpc.fastnear.com";
const POOL = process.env.POOL ?? "pool.blindbook-dev.testnet";
const OWNER = process.env.OWNER ?? "blindbook-dev.testnet";
const CRANK = process.env.CRANK ?? "blindbook-crank-e4c061.testnet";
const SFX = "bbcced2c";
const VERIFIER = `intents-${SFX}.testnet`;
const SELLER = `seller-${SFX}.testnet`;
const BUYER = `buyer-${SFX}.testnet`;
const VENUE = `venue-${SFX}.testnet`;
const T_NEAR = `nep141:tnear-${SFX}.testnet`;
const T_USDC = `nep141:tusdc-${SFX}.testnet`;
const MARKET = "tnear-tusdc";
const E24 = 10n ** 24n;
const BUCKETS = [2048, 4096, 8192, 16384, 32768, 65536];
const EVIDENCE = new URL("./evidence/", import.meta.url);
mkdirSync(EVIDENCE, { recursive: true });

const provider = new JsonRpcProvider({ url: RPC });
const te = new TextEncoder();
const b64 = (u: Uint8Array) => Buffer.from(u).toString("base64");

// ---------------------------------------------------------------- keys + rpc

const nearKey = (id: string) =>
  JSON.parse(readFileSync(join(homedir(), ".near-credentials", "testnet", `${id}.json`), "utf8")).private_key as string;
const seedOf = (id: string) => bs58.decode(nearKey(id).replace("ed25519:", "")).slice(0, 32);
const venueSeed = () =>
  Buffer.from(JSON.parse(readFileSync(join(homedir(), ".near-credentials", `blindbook-spike-${SFX}.json`), "utf8")).venueSeedHex, "hex");

async function rpc(method: string, params: unknown): Promise<any> {
  for (let i = 0; ; i++) {
    const r = await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const j: any = await r.json().catch(() => ({ error: { message: `http ${r.status}` } }));
    if (!j.error) return j.result;
    if (i >= 4) throw new Error(`${method}: ${JSON.stringify(j.error).slice(0, 400)}`);
    await sleep(1500);
  }
}
async function view<T = any>(contract: string, method: string, args: unknown = {}): Promise<T> {
  const res = await rpc("query", { request_type: "call_function", finality: "optimistic", account_id: contract, method_name: method, args_base64: b64(te.encode(JSON.stringify(args))) });
  return JSON.parse(Buffer.from(res.result).toString());
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function send(signer: string, receiver: string, method: string, args: object, gasT: bigint, deposit: bigint): Promise<string> {
  const acct = new Account(signer, provider, KeyPairSigner.fromSecretKey(nearKey(signer) as any));
  const res: any = await acct.signAndSendTransaction({
    receiverId: receiver,
    actions: [actions.functionCall(method, args, gasT * 10n ** 12n, deposit)],
    waitUntil: "INCLUDED_FINAL" as any,
    throwOnFailure: false,
  });
  return res.transaction_outcome?.id ?? res.transaction?.hash;
}

interface Receipt { id: string; executor: string; method?: string; gasBurnt: number; status: string; logs: string[] }
async function txReport(hash: string, sender: string) {
  const r = await rpc("EXPERIMENTAL_tx_status", { tx_hash: hash, sender_account_id: sender, wait_until: "FINAL" });
  const methods = new Map<string, string>();
  for (const rc of r.receipts ?? []) {
    const fc = rc.receipt?.Action?.actions?.find((a: any) => a.FunctionCall)?.FunctionCall;
    const tr = rc.receipt?.Action?.actions?.find((a: any) => a.Transfer);
    methods.set(rc.receipt_id, fc ? fc.method_name : tr ? "transfer" : "data/refund");
  }
  const receipts: Receipt[] = r.receipts_outcome.map((o: any) => ({
    id: o.id,
    executor: o.outcome.executor_id,
    method: methods.get(o.id),
    gasBurnt: Number(o.outcome.gas_burnt) / 1e12,
    status: Object.keys(o.outcome.status)[0],
    logs: o.outcome.logs,
  }));
  const total = Number(r.transaction_outcome.outcome.gas_burnt) / 1e12 + receipts.reduce((a, x) => a + x.gasBurnt, 0);
  return { hash, status: Object.keys(r.status)[0], totalTgas: +total.toFixed(2), receipts };
}

function printReport(rep: Awaited<ReturnType<typeof txReport>>) {
  console.log(`  tx ${rep.hash}  status=${rep.status}  gas burnt total=${rep.totalTgas} TGas`);
  for (const r of rep.receipts) {
    if (r.method === "data/refund" && r.gasBurnt < 0.3) continue;
    console.log(`   - ${r.executor.padEnd(32)} ${String(r.method).padEnd(24)} ${r.gasBurnt.toFixed(2).padStart(7)} TGas  ${r.status}`);
    for (const l of r.logs) console.log(`       log: ${l.slice(0, 220)}`);
  }
}

async function waitBatch(id: number, terminal = ["completed", "settled", "partially_settled", "settlement_failed", "failed", "rejected_signature", "invalid_result", "unpinned"]) {
  const t0 = Date.now();
  for (;;) {
    const b = await view(POOL, "get_batch", { id: String(id) });
    if (b && terminal.includes(b.status)) return { batch: b, seconds: (Date.now() - t0) / 1000 };
    if (Date.now() - t0 > 240_000) throw new Error(`batch ${id} still ${b?.status}`);
    await sleep(2000);
  }
}

async function crank(method: "run_batch" | "run_keys") {
  const cfg = await view(POOL, "get_config");
  const id = cfg.next_batch_id as number;
  const gasT = BigInt(cfg.run_gas) / 10n ** 12n + 15n;
  console.log(`${method} by ${CRANK}: batch ${id}, deposit ${cfg.run_deposit} yocto, gas ${gasT} TGas`);
  const hash = await send(CRANK, POOL, method, {}, gasT, BigInt(cfg.run_deposit));
  const { batch, seconds } = await waitBatch(id);
  await sleep(4000); // let the settlement receipts finalize
  const rep = await txReport(hash, CRANK);
  printReport(rep);
  const final = await view(POOL, "get_batch", { id: String(id) });
  console.log(`  batch ${id}: ${batch.status} -> ${final.status} after ${seconds}s`, final.error ? `(${final.error})` : "");
  return { id, hash, gasT: Number(gasT), report: rep, batch: final };
}

// ---------------------------------------------------------------- intents (subset of the defuse spike helper)

const u32le = (n: number) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n, true); return b; };
const concat = (...p: Uint8Array[]) => { const o = new Uint8Array(p.reduce((n, x) => n + x.length, 0)); let i = 0; for (const x of p) { o.set(x, i); i += x.length; } return o; };
const borshStr = (s: string) => { const b = te.encode(s); return concat(u32le(b.length), b); };

function makeNonce(saltHex: string, deadline: Date): string {
  const dl = new Uint8Array(8);
  new DataView(dl.buffer).setBigInt64(0, BigInt(deadline.getTime()) * 1_000_000n, true);
  return b64(concat(Buffer.from("5628f6c6", "hex"), new Uint8Array([0]), Buffer.from(saltHex, "hex"), dl, crypto.getRandomValues(new Uint8Array(15))));
}

function signNep413(signerId: string, seed: Uint8Array, salt: string, deadline: Date, intents: object[]) {
  const nonce = makeNonce(salt, deadline);
  const message = JSON.stringify({ signer_id: signerId, deadline: deadline.toISOString(), intents });
  const hash = sha256(concat(u32le(2147484061), borshStr(message), Buffer.from(nonce, "base64"), borshStr(VERIFIER), new Uint8Array([0])));
  return {
    standard: "nep413",
    payload: { message, nonce, recipient: VERIFIER },
    public_key: "ed25519:" + bs58.encode(ed25519.getPublicKey(seed)),
    signature: "ed25519:" + bs58.encode(ed25519.sign(hash, seed)),
  };
}

// Venue closure under the verifier's protocol fee (TokenDiff::closure_many).
const PIPS = 1_000_000n;
const divCeil = (a: bigint, b: bigint) => { const q = a / b; return a % b !== 0n && (a > 0n) === (b > 0n) ? q + 1n : q; };
const divFloor = (a: bigint, b: bigint) => { const q = a / b; return a % b !== 0n && (a > 0n) !== (b > 0n) ? q - 1n : q; };
function closure(diffs: Record<string, bigint>[], fee: bigint) {
  const sup: Record<string, bigint> = {};
  for (const d of diffs) for (const [t, v] of Object.entries(d)) sup[t] = (sup[t] ?? 0n) + (v >= 0n ? v : divCeil(v * (PIPS - fee), PIPS));
  const out: Record<string, bigint> = {};
  for (const [t, s] of Object.entries(sup)) { let c = -s; if (c < 0n) c = divFloor(c * PIPS, PIPS - fee); if (c !== 0n) out[t] = c; }
  return out;
}
const diff = (d: Record<string, bigint>) => ({ intent: "token_diff", diff: Object.fromEntries(Object.entries(d).map(([k, v]) => [k, v.toString()])) });
const transfer = (to: string, t: Record<string, bigint>) => ({ intent: "transfer", receiver_id: to, tokens: Object.fromEntries(Object.entries(t).map(([k, v]) => [k, v.toString()])) });

/** One crossing lot: seller sells 1 tNEAR at >= 2.50, buyer buys at <= 2.60, venue closes and rebates. */
async function crossingPayloads() {
  const fee = BigInt(await view<number>(VERIFIER, "fee"));
  const salt = await view<string>(VERIFIER, "current_salt");
  const deadline = new Date(Date.now() + 15 * 60_000);
  const sell = { [T_NEAR]: -1n * E24, [T_USDC]: 2_500_000n };
  const buy = { [T_NEAR]: 1n * E24, [T_USDC]: -2_600_000n };
  const close = closure([sell, buy], fee);
  const surplus = close[T_USDC] ?? 0n;
  const keep = 20_000n; // venue fee: 0.02 tUSDC
  const rebate = (surplus - keep) / 2n;
  const signed = [
    signNep413(SELLER, seedOf(SELLER), salt, deadline, [diff(sell)]),
    signNep413(BUYER, seedOf(BUYER), salt, deadline, [diff(buy)]),
    signNep413(VENUE, venueSeed(), salt, deadline, [diff(close), transfer(SELLER, { [T_USDC]: rebate }), transfer(BUYER, { [T_USDC]: rebate })]),
  ];
  const sim: any = await view(VERIFIER, "simulate_intents", { signed });
  if (sim.invariant_violated) throw new Error("simulate: invariant violated " + JSON.stringify(sim.invariant_violated));
  const summary = {
    market: MARKET, mid: { quote: "2550000", base: E24.toString() }, lot_mid_quote: "2550000", matched_lots: 1,
    base_volume: E24.toString(), quote_volume: "2550000", rebated: (2n * rebate).toString(), venue_fee: (surplus - 2n * rebate).toString(),
    quotes: ["echo:no-1click-on-testnet"], sub_batches: 1, dropped: [], skipped: null,
  };
  console.log(`verifier fee=${fee} pips salt=${salt}; closure ${JSON.stringify(Object.fromEntries(Object.entries(close).map(([k, v]) => [k.split(":")[1], v.toString()])))}; rebate ${rebate} each; simulate ok (${sim.intents_executed.length} intents)`);
  return { signed, summary };
}

/** Same framing and bucket sizes as a sealed order, plaintext in the clear (pool-echo only). */
function echoEnvelope(instruction: object): string {
  const body = te.encode(JSON.stringify(instruction));
  const bucket = BUCKETS.find((b) => b >= body.length + 4);
  if (!bucket) throw new Error("instruction too large");
  const plain = new Uint8Array(bucket);
  plain.set(u32le(body.length), 0);
  plain.set(body, 4);
  return b64(concat(new Uint8Array([1]), new Uint8Array(32 + 24), plain, new Uint8Array(16)));
}

async function balances() {
  const out: Record<string, { tnear: string; tusdc: string }> = {};
  for (const a of [SELLER, BUYER, VENUE]) {
    const [n, u] = await view<string[]>(VERIFIER, "mt_batch_balance_of", { account_id: a, token_ids: [T_NEAR, T_USDC] });
    out[a] = { tnear: n, tusdc: u };
  }
  return out;
}

async function submit(sealed: string, who = SELLER) {
  const len = Buffer.from(sealed, "base64").length;
  const deposit = BigInt((await view(POOL, "get_order_deposit", { bytes: len })) as string);
  const next = (await view(POOL, "get_config")).next_seq as number;
  const hash = await send(who, POOL, "submit_order", { market: MARKET, sealed_b64: sealed }, 30n, deposit);
  const rep = await txReport(hash, who);
  console.log(`submit_order by ${who}: order ${next}, ${len} bytes, attached ${deposit} yocto`);
  printReport(rep);
  return { id: next, hash, report: rep, bytes: len };
}

function save(name: string, data: unknown) {
  writeFileSync(new URL(`${name}.json`, EVIDENCE), JSON.stringify(data, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2));
}

// ---------------------------------------------------------------- flows

const cmd = process.argv[2];
if (cmd === "keys") {
  const r = await crank("run_keys");
  console.log("  reported keys:", JSON.stringify(r.batch.keys));
  console.log("  pool matcher_key:", await view(POOL, "get_matcher_key"), " venue_key:", (await view(POOL, "get_config")).venue_key);
  save(`keys-${r.id}-${r.batch.status}`, r);
} else if (cmd === "pin") {
  const batches: any[] = await view(POOL, "get_batches", { limit: 50 });
  const last = batches.reverse().find((b) => b.op === "keys" && b.keys);
  if (!last) throw new Error("no keys run recorded");
  console.log(`pinning result_key ${last.keys.result_key} reported by batch ${last.id} (${last.status})`);
  const hash = await send(OWNER, POOL, "set_result_key", { key: last.keys.result_key }, 30n, 0n);
  const rep = await txReport(hash, OWNER);
  printReport(rep);
  save(`pin-${last.id}`, rep);
} else if (cmd === "settle") {
  const { signed, summary } = await crossingPayloads();
  const sealed = echoEnvelope({ echo: 1, settle: [signed], batches: [summary] });
  const before = await balances();
  const sub = await submit(sealed);
  const r = await crank("run_batch");
  const after = await balances();
  const deltas = Object.fromEntries(Object.keys(before).map((a) => [a, {
    tnear: (BigInt(after[a].tnear) - BigInt(before[a].tnear)).toString(),
    tusdc: (BigInt(after[a].tusdc) - BigInt(before[a].tusdc)).toString(),
  }]));
  console.log("balance deltas on the verifier:", JSON.stringify(deltas, null, 1));
  console.log("pool stats:", JSON.stringify(await view(POOL, "get_stats")));
  save(`settle-${r.id}`, { submit: sub, run: r, before, after, deltas });
} else if (cmd === "forge") {
  const mode = process.argv[3] ?? "wrong_key";
  const { signed, summary } = await crossingPayloads();
  const sealed = echoEnvelope({ echo: 1, settle: [signed], batches: [summary], forge: mode });
  const before = await balances();
  const sub = await submit(sealed);
  const r = await crank("run_batch");
  const after = await balances();
  const unchanged = JSON.stringify(before) === JSON.stringify(after);
  const order = await view(POOL, "get_order", { id: String(sub.id) });
  console.log(`forged (${mode}): status ${r.batch.status}; order ${sub.id} still pending: ${!!order}; verifier balances unchanged: ${unchanged}`);
  const hash = await send(SELLER, POOL, "cancel_order", { id: String(sub.id) }, 30n, 0n);
  const cancel = await txReport(hash, SELLER);
  console.log("cancel_order:");
  printReport(cancel);
  save(`forge-${mode}-${r.id}`, { submit: sub, run: r, unchanged, cancel });
} else if (cmd === "real") {
  // Against the real matcher: two sealed orders placed with @blindbook/client,
  // then one crank. Seller limit low, buyer limit high, so they cross at the mid.
  const { Blindbook } = await import("../../../packages/client/dist/index.js");
  const { TESTNET } = await import("../../../packages/sdk/dist/index.js");
  const [sellerId, buyerId] = (process.argv[3] ?? "house.blindbook-dev.testnet,demo.blindbook-dev.testnet").split(",");
  const sellLimit = BigInt(process.argv[4] ?? "500000"); // >= 0.5 tUSDC per tNEAR
  const buyLimit = BigInt(process.argv[5] ?? "10000000"); // <= 10 tUSDC per tNEAR
  const client = (id: string) => new Blindbook({ network: TESTNET, accountId: id, secretKey: nearKey(id) });
  const who = [sellerId, buyerId, "venue.blindbook-dev.testnet"];
  const bal = async () => {
    const out: Record<string, { tnear: string; tusdc: string }> = {};
    for (const a of who) {
      const [n, u] = await view<string[]>(VERIFIER, "mt_batch_balance_of", { account_id: a, token_ids: [T_NEAR, T_USDC] });
      out[a] = { tnear: n, tusdc: u };
    }
    return out;
  };
  const venueKey = (await view(POOL, "get_config")).venue_key;
  const registered = await view<boolean>(VERIFIER, "has_public_key", { account_id: "venue.blindbook-dev.testnet", public_key: venueKey });
  console.log(`matcher key ${await view(POOL, "get_matcher_key")}; venue key ${venueKey} registered on verifier: ${registered}`);
  const before = await bal();
  const sell = await client(sellerId).placeOrder({ market: MARKET, side: "sell", limits: [sellLimit], ttlMinutes: 60 });
  const buy = await client(buyerId).placeOrder({ market: MARKET, side: "buy", limits: [buyLimit], ttlMinutes: 60 });
  for (const [name, o] of [["sell", sell], ["buy", buy]] as const) {
    console.log(`${name} order ${o.orderId} by ${name === "sell" ? sellerId : buyerId}: tx ${o.tx.hash} ok=${o.tx.ok}${o.tx.error ? " " + o.tx.error : ""}`);
  }
  const r = await crank("run_batch");
  const after = await bal();
  const deltas = Object.fromEntries(who.map((a) => [a, {
    tnear: (BigInt(after[a].tnear) - BigInt(before[a].tnear)).toString(),
    tusdc: (BigInt(after[a].tusdc) - BigInt(before[a].tusdc)).toString(),
  }]));
  const b = r.batch;
  console.log("batch:", JSON.stringify({ status: b.status, error: b.error, ingested: b.ingested, rejected: b.rejected, deferred: b.deferred,
    input_hash: b.input_hash, result_hash: b.result_hash, markets: b.markets, sub_batches: b.sub_batches, settled: b.settled, book: b.book }, null, 1));
  console.log("balance deltas on the verifier:", JSON.stringify(deltas, null, 1));
  console.log("lots consumed:", { sell: await client(sellerId).lotStatus(sell.drafts), buy: await client(buyerId).lotStatus(buy.drafts) });
  save(`real-${r.id}`, { sell: { id: sell.orderId, tx: sell.tx.hash }, buy: { id: buy.orderId, tx: buy.tx.hash }, run: r, before, after, deltas });
} else if (cmd === "crank") {
  const r = await crank("run_batch");
  console.log("batch:", JSON.stringify({ status: r.batch.status, error: r.batch.error, markets: r.batch.markets, settled: r.batch.settled }));
  save(`crank-${r.id}`, r);
} else {
  console.log("usage: tsx e2e.ts keys|pin|settle|forge [wrong_key|tamper]|real [seller,buyer] [sellLimit] [buyLimit]|crank");
}
