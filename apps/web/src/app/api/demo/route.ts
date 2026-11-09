import { type DemoSide, demoAvailable, runDemo } from "@/lib/server/demo";

export const maxDuration = 60;

export async function GET() {
  return Response.json({ available: demoAvailable() });
}

export async function POST(request: Request) {
  const body = (await request.json().catch(() => ({}))) as { side?: string };
  const side: DemoSide = body.side === "buy" ? "buy" : "sell";
  const result = await runDemo(side);
  return Response.json(result, { status: result.ok ? 200 : result.reason?.includes("try again") ? 429 : 502 });
}
