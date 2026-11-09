/**
 * Independent verification of one Blindbook batch from public sources only:
 * the pool's on-chain record and config, the NEAR Intents verifier, and
 * OutLayer's Intel TDX attestation for the enclave run.
 */
import { type Network, rpcView } from "@blindbook/sdk";

export interface Check {
  id: string;
  label: string;
  ok: boolean | null;
  detail: string;
  link?: string;
}

export interface BatchMarket {
  market: string;
  mid: { quote: string; base: string } | null;
  lot_mid_quote: string;
  matched_lots: number;
  base_volume: string;
  quote_volume: string;
  rebated: string;
  venue_fee: string;
  quotes: string[];
  sub_batches: number;
  skipped: string | null;
}

export interface BatchRecord {
  id: string;
  op: string;
  cranker: string;
  orders: string[];
  input_bytes: number;
  input_hash: string;
  requested_block: number;
  status: string;
  error: string | null;
  result_hash: string | null;
  ingested: string[];
  rejected: { id: string; reason: string }[];
  markets: BatchMarket[];
  sub_batches: number;
  dispatched: number;
  settled: boolean[];
}

interface PoolConfig {
  project_id: string;
  version_key: string;
  result_key: string | null;
  venue_account: string;
  venue_key: string | null;
  verifier: string;
}

interface Attestation {
  request_id: string;
  transaction_hash: string;
  executed_wasm_sha256: string;
  input_hash: string;
  output_hash: string;
  project_id: string;
  worker_measurement: string;
  tdx_quote?: string;
}

export const outlayerApiFor = (network: Network) =>
  network.networkId === "mainnet" ? "https://api.outlayer.ai" : "https://testnet-api.outlayer.ai";

/** The OutLayer request id a crank transaction dispatched, read from its logs. */
export async function requestIdFromTx(network: Network, txHash: string, sender: string): Promise<string | null> {
  const res = await fetch(network.rpc, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: "blindbook",
      method: "EXPERIMENTAL_tx_status",
      params: { tx_hash: txHash, sender_account_id: sender, wait_until: "FINAL" },
    }),
  }).catch(() => null);
  if (!res?.ok) return null;
  const j = (await res.json()) as { result?: { receipts_outcome: { outcome: { logs: string[] } }[] } };
  for (const r of j.result?.receipts_outcome ?? []) {
    for (const log of r.outcome.logs) {
      // the id sits inside a JSON string in the event, so its quotes may be escaped
      const m = /request_id\\?"\s*:\s*(\d+)/.exec(log);
      if (m) return m[1];
    }
  }
  return null;
}

export async function verifyBatch(
  network: Network,
  batchId: string,
  requestId?: string,
): Promise<{ batch: BatchRecord | null; checks: Check[] }> {
  const view = <T>(method: string, args: object = {}) => rpcView<T>(network.rpc, network.pool, method, args);
  const [batch, cfg] = await Promise.all([
    view<BatchRecord | null>("get_batch", { id: batchId }).catch(() => null),
    view<PoolConfig>("get_config"),
  ]);
  const checks: Check[] = [];
  if (!batch) {
    return { batch: null, checks: [{ id: "batch", label: "Batch exists", ok: false, detail: `no batch ${batchId} on ${network.pool}` }] };
  }
  const outlayer = outlayerApiFor(network);

  checks.push({
    id: "result-signature",
    label: "Matcher result signature verified on-chain",
    ok: batch.status !== "rejected_signature" && batch.result_hash !== null,
    detail: `the pool acts only on results signed by the pinned key ${cfg.result_key ?? "(unset)"}; result sha256 ${batch.result_hash ?? "none"}`,
  });

  const crossed = batch.markets.some((m) => m.matched_lots > 0);
  checks.push({
    id: "settlement",
    label: "Both legs settled in execute_intents",
    ok: crossed ? batch.settled.length > 0 && batch.settled.every(Boolean) : null,
    detail: crossed
      ? `${batch.dispatched} sub-batch(es) sent to ${cfg.verifier}, ${batch.settled.filter(Boolean).length} settled`
      : "nothing crossed in this batch",
  });

  const quotes = batch.markets.flatMap((m) => m.quotes);
  checks.push({
    id: "quotes",
    label: "Reference price from signed 1Click quotes",
    ok: quotes.length > 0 ? true : null,
    detail: quotes.length > 0 ? `the enclave verified ${quotes.length} ed25519 quote signatures: ${quotes.join(", ")}` : "no market was priced",
  });

  if (requestId) {
    const att = await fetch(`${outlayer}/attestations/by-request/${requestId}`)
      .then((r) => (r.ok ? (r.json() as Promise<Attestation>) : null))
      .catch(() => null);
    if (!att) {
      checks.push({ id: "attestation", label: "Intel TDX attestation for the run", ok: null, detail: "attestation unavailable right now" });
    } else {
      checks.push({
        id: "attestation",
        label: "Intel TDX attestation for the run",
        ok: Boolean(att.tdx_quote) && att.project_id === cfg.project_id,
        detail: `worker ${att.worker_measurement.slice(0, 16)}…, project ${att.project_id}`,
        link: `${outlayer}/attestations/by-request/${requestId}`,
      });
      checks.push({
        id: "code",
        label: "Enclave ran the pinned matcher build",
        ok: att.executed_wasm_sha256 === cfg.version_key,
        detail: `executed ${att.executed_wasm_sha256} · pinned ${cfg.version_key}`,
      });
      checks.push({
        id: "input",
        label: "Enclave input is the batch the pool sent",
        ok: att.input_hash === batch.input_hash,
        detail: `attested ${att.input_hash} · on-chain ${batch.input_hash}`,
      });
    }
  }

  const hasKey = cfg.venue_key
    ? await rpcView<boolean>(network.rpc, cfg.verifier, "has_public_key", {
        account_id: cfg.venue_account,
        public_key: cfg.venue_key,
      }).catch(() => false)
    : false;
  checks.push({
    id: "venue-key",
    label: "Venue key registered on the verifier",
    ok: hasKey,
    detail: `${cfg.venue_account} signs only the closure and the rebates, with ${cfg.venue_key ?? "no key"}`,
  });

  return { batch, checks };
}
