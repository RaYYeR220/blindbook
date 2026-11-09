/**
 * Re-verify a Blindbook batch from public sources.
 *
 *   pnpm verify:batch <batch-id> [--tx <crank tx hash> --sender <cranker>] [--request <outlayer id>] [--mainnet]
 *
 * Checks the matcher's result signature was accepted on-chain, both legs
 * settled on the verifier, the price came from signed 1Click quotes, the
 * enclave ran the pinned build on exactly the input the pool sent (Intel TDX
 * attestation), and the venue key is registered on the verifier.
 */
import { requestIdFromTx, verifyBatch } from "../packages/client/src/index.js";
import { MAINNET, TESTNET } from "../packages/sdk/src/index.js";

const args = process.argv.slice(2);
const flag = (name: string) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : undefined;
};
const batch = args.find((a) => /^\d+$/.test(a));
if (!batch) {
  console.error("usage: pnpm verify:batch <batch-id> [--tx <hash> --sender <account>] [--request <id>] [--mainnet]");
  process.exit(2);
}
const network = args.includes("--mainnet") ? MAINNET : TESTNET;
const tx = flag("tx");
const sender = flag("sender");
const requestId = flag("request") ?? (tx && sender ? await requestIdFromTx(network, tx, sender) : undefined) ?? undefined;
const { checks } = await verifyBatch(network, batch, requestId);
console.log(`batch ${batch} on ${network.pool}${requestId ? ` · OutLayer request ${requestId}` : ""}\n`);
for (const c of checks) {
  const mark = c.ok === true ? "PASS" : c.ok === false ? "FAIL" : "n/a ";
  console.log(`${mark}  ${c.label}\n      ${c.detail}${c.link ? `\n      ${c.link}` : ""}`);
}
process.exit(checks.some((c) => c.ok === false) ? 1 : 0);
