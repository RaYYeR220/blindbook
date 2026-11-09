import { forget } from "@/lib/server/pool";
import { relayCrank } from "@/lib/server/relay";

export const maxDuration = 60;

/** Anyone may crank a batch; this route does it with the relayer's gas. */
export async function POST() {
  try {
    const r = await relayCrank();
    forget();
    return Response.json(r, { status: r.ok ? 200 : 429 });
  } catch (e) {
    return Response.json({ ok: false, error: (e as Error).message }, { status: 400 });
  }
}
