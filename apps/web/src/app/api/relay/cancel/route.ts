import type { MultiPayload } from "@blindbook/sdk";
import { relayCancel } from "@/lib/server/relay";

export const maxDuration = 30;

export async function POST(request: Request) {
  const body = (await request.json().catch(() => ({}))) as { tickets?: MultiPayload[] };
  try {
    return Response.json(await relayCancel(body.tickets ?? []));
  } catch (e) {
    return Response.json({ ok: false, error: (e as Error).message }, { status: 400 });
  }
}
