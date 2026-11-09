import { network } from "@/lib/server/env";
import { type PoolStats, poolView } from "@/lib/server/pool";

export async function GET() {
  const [config, stats] = await Promise.all([poolView("get_config"), poolView<PoolStats>("get_stats")]);
  return Response.json({ network: network.networkId, pool: network.pool, verifier: network.verifier, config, stats });
}
