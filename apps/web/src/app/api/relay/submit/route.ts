import { relaySubmit } from "@/lib/server/relay";

export const maxDuration = 30;

export async function POST(request: Request) {
  const body = (await request.json().catch(() => ({}))) as { market?: string; sealed_b64?: string };
  if (!body.market || !body.sealed_b64) return Response.json({ error: "market and sealed_b64 required" }, { status: 400 });
  try {
    return Response.json(await relaySubmit(body.market, body.sealed_b64));
  } catch (e) {
    return Response.json({ ok: false, error: (e as Error).message }, { status: 400 });
  }
}
