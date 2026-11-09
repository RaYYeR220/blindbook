import { fetchSalt, isNonceUsed, rpcView } from "@blindbook/sdk";
import { NET } from "./net";
import { readJson, writeJson } from "./storage";
import type { BatchView, PoolConfigView } from "./types";

/**
 * Direct, read-only RPC from the browser. The app routes cache pool views for
 * a few seconds; these read the chain itself when the UI needs to see a
 * change as it lands (demo progress, lot status).
 */

export const poolConfig = () => rpcView<PoolConfigView>(NET.rpc, NET.pool, "get_config");

export const getBatch = (id: string | number) =>
  rpcView<BatchView | null>(NET.rpc, NET.pool, "get_batch", { id: String(id) });

export const currentSalt = () => fetchSalt(NET.rpc, NET.verifier);

export const nonceUsed = (account: string, nonce: string) => isNonceUsed(NET.rpc, NET.verifier, account, nonce);

interface RpcEnvelope<T> {
  result?: T;
  error?: unknown;
}

async function rpc<T>(method: string, params: unknown): Promise<T | null> {
  const res = await fetch(NET.rpc, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: "blindbook", method, params }),
  }).catch(() => null);
  if (!res?.ok) return null;
  const j = (await res.json().catch(() => null)) as RpcEnvelope<T> | null;
  return j?.result ?? null;
}

interface BlockView {
  chunks: { chunk_hash: string; height_included: number }[];
}
interface ChunkView {
  transactions: {
    hash: string;
    signer_id: string;
    receiver_id: string;
    actions: ({ FunctionCall?: { method_name: string } } | string)[];
  }[];
}

/** Crank transactions already found, by batch id. Public data only. */
const CRANKS_KEY = `bb:cranks:v1:${NET.pool}`;
type CrankMap = Record<string, { tx: string; sender: string }>;

export function rememberCrank(batchId: string | number, tx: string, sender: string) {
  const map = readJson<CrankMap>(CRANKS_KEY, {});
  map[String(batchId)] = { tx, sender };
  const ids = Object.keys(map).sort((a, b) => Number(b) - Number(a));
  writeJson(CRANKS_KEY, Object.fromEntries(ids.slice(0, 64).map((k) => [k, map[k]])));
}

export const knownCrank = (batchId: string | number) => readJson<CrankMap>(CRANKS_KEY, {})[String(batchId)] ?? null;

/**
 * The `run_batch` transaction that opened a batch. The pool records the
 * block its call executed in and who cranked; the transaction itself sits in
 * that block or one or two before it, in a chunk signed by the cranker.
 */
export async function findCrankTx(batch: Pick<BatchView, "id" | "cranker" | "requested_block">): Promise<string | null> {
  const known = knownCrank(batch.id);
  if (known) return known.tx;
  for (const height of [batch.requested_block - 1, batch.requested_block - 2, batch.requested_block]) {
    const block = await rpc<BlockView>("block", { block_id: height });
    if (!block) continue;
    const chunks = await Promise.all(
      block.chunks.filter((c) => c.height_included === height).map((c) => rpc<ChunkView>("chunk", { chunk_id: c.chunk_hash })),
    );
    for (const chunk of chunks) {
      for (const tx of chunk?.transactions ?? []) {
        const isCrank = tx.actions.some((a) => typeof a === "object" && a.FunctionCall?.method_name === "run_batch");
        if (tx.signer_id === batch.cranker && tx.receiver_id === NET.pool && isCrank) {
          rememberCrank(batch.id, tx.hash, batch.cranker);
          return tx.hash;
        }
      }
    }
  }
  return null;
}
