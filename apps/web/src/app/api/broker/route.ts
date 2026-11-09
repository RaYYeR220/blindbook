import { brokerAvailable, BROKER_MODEL, plan } from "@/lib/server/broker";

export const maxDuration = 60;

export async function GET() {
  return Response.json({ available: brokerAvailable(), model: BROKER_MODEL });
}

/** Turn a plain-language mandate into a lot plan with a model running in a TEE. */
export async function POST(request: Request) {
  if (!brokerAvailable()) return Response.json({ ok: false, error: "the private broker is offline" }, { status: 503 });
  const body = (await request.json().catch(() => ({}))) as { mandate?: string; mid?: number; lot?: string; market?: string };
  const mandate = (body.mandate ?? "").trim().slice(0, 600);
  if (mandate.length < 6) return Response.json({ ok: false, error: "describe the block in a sentence" }, { status: 400 });
  const result = await plan(mandate, { mid: Number(body.mid) || 0, lotLabel: body.lot ?? "1 lot", market: body.market ?? "NEAR/USDC" });
  return Response.json(result, { status: result.ok ? 200 : 502 });
}
