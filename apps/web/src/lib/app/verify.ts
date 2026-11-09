import { api } from "./api";
import { findCrankTx, rememberCrank } from "./chain";
import type { BatchStatus, BatchView, VerifyResponse } from "./types";

export interface Verification extends VerifyResponse {
  batchId: string;
  /** the run_batch transaction, when it could be found */
  tx: string | null;
  sender: string;
}

/** Finished verifications, by batch id. A settled batch's checks don't change. */
const cache = new Map<string, Promise<Verification>>();
const RUNNING = new Set<BatchStatus>(["requested", "settling"]);

/**
 * Run the seven public checks for a batch. Without the crank transaction the
 * attestation checks can't be tied to the run, so it is looked up first.
 */
export function verifyBatch(batch: Pick<BatchView, "id" | "cranker" | "requested_block">, tx?: string | null): Promise<Verification> {
  const hit = cache.get(batch.id);
  if (hit) return hit;
  const p = (async () => {
    if (tx) rememberCrank(batch.id, tx, batch.cranker);
    const crank = tx ?? (await findCrankTx(batch));
    const r = await api.verify(batch.id, crank, crank ? batch.cranker : null);
    return { ...r, batchId: batch.id, tx: crank, sender: batch.cranker };
  })();
  cache.set(batch.id, p);
  // keep only final, complete answers: a running batch, a missing crank tx or a failure is retried next time
  p.then((v) => (v.tx && v.batch && !RUNNING.has(v.batch.status) ? undefined : cache.delete(batch.id))).catch(() =>
    cache.delete(batch.id),
  );
  return p;
}

/** The seven check ids in the order they are shown. */
export const CHECK_ORDER = ["result-signature", "settlement", "quotes", "attestation", "code", "input", "venue-key"] as const;

/** Short names for the checks, for dense lists. The full label stays available as a title. */
export const CHECK_SHORT: Record<string, string> = {
  "result-signature": "Result signature checked on-chain",
  settlement: "Both legs settled in one call",
  quotes: "Mid from signed 1Click quotes",
  attestation: "Intel TDX attestation",
  code: "Pinned matcher build ran",
  input: "Enclave input = pool's batch",
  "venue-key": "Venue key registered",
  batch: "Batch exists",
};
