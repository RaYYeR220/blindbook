import type { NextRequest } from "next/server";
import { type BatchRecord, poolView } from "@/lib/server/pool";

export async function GET(request: NextRequest) {
  const limit = Math.min(50, Math.max(1, Number(request.nextUrl.searchParams.get("limit") ?? 20)));
  const batches = await poolView<BatchRecord[]>("get_batches", { from: null, limit });
  // newest first; only public post-trade records, never resting orders
  return Response.json({ batches: [...batches].reverse() });
}
