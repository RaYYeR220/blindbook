import { requestIdFromTx, verifyBatch } from "@blindbook/client";
import type { NextRequest } from "next/server";
import { network } from "@/lib/server/env";

/**
 * Re-check one batch from public sources. Pass the batch id, and the crank
 * transaction (hash + signer) or the OutLayer request id to also check the
 * enclave attestation.
 */
export async function GET(request: NextRequest) {
  const q = request.nextUrl.searchParams;
  const batch = q.get("batch");
  if (!batch || !/^\d+$/.test(batch)) return Response.json({ error: "batch id required" }, { status: 400 });
  const tx = q.get("tx");
  const sender = q.get("sender");
  const requestId = q.get("request") ?? (tx && sender ? await requestIdFromTx(network, tx, sender) : null);
  const result = await verifyBatch(network, batch, requestId ?? undefined);
  return Response.json({ ...result, request_id: requestId });
}
