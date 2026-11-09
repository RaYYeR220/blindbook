import { drip, faucetAvailable } from "@/lib/server/faucet";

export const maxDuration = 30;

export async function GET() {
  return Response.json({ available: faucetAvailable() });
}

export async function POST(request: Request) {
  const body = (await request.json().catch(() => ({}))) as { account?: string };
  try {
    return Response.json({ ok: true, ...(await drip(body.account ?? "")) });
  } catch (e) {
    return Response.json({ ok: false, error: (e as Error).message }, { status: 400 });
  }
}
