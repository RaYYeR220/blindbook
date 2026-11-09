import "server-only";
import { secp256k1 } from "@noble/curves/secp256k1";
import { sha256 } from "@noble/hashes/sha256";
import { keccak_256 } from "@noble/hashes/sha3";
import { bytesToHex, hexToBytes, randomBytes } from "@noble/hashes/utils";

/**
 * Private broker: turns a plain-language mandate ("unload 40 lots this week,
 * never below 3.40") into a lot plan, using a model that runs inside a TEE on
 * NEAR AI Cloud. A trading intention is the most sensitive thing a trader has,
 * so it only ever goes to an attested model, and the answer comes back with
 * the evidence that it did. The mandate is never logged or stored.
 */

const BASE = "https://cloud-api.near.ai";
const NRAS = "https://nras.attestation.nvidia.com/v3/attest/gpu";
export const BROKER_MODEL = process.env.NEARAI_MODEL ?? "z-ai/glm-5.3-flash";

export const brokerAvailable = () => Boolean(process.env.NEARAI_API_KEY);

export interface BrokerCheck {
  id: string;
  label: string;
  ok: boolean | null;
  detail: string;
}

export interface LotPlan {
  side: "sell" | "buy";
  lots: number;
  limit_price: string;
  ladder_step_bps: number;
  min_lots: number;
  ttl_minutes: number;
  rationale: string;
}

const SYSTEM = `You are a block-trading broker for Blindbook, a dark pool where orders are split into equal lots, each signed at its own limit, and crossed only at a verified midpoint.
Turn the trader's mandate into a plan. Reply with ONLY a JSON object:
{"side":"sell"|"buy","lots":<int 1-10>,"limit_price":"<decimal, quote per base>","ladder_step_bps":<int 0-100>,"min_lots":<int>=1>,"ttl_minutes":<int 10-1440>,"rationale":"<one sentence>"}
Rules: never set a limit better for the trader than the mandate allows; a sell limit is the lowest price accepted, a buy limit the highest. If the mandate names no limit, use the reference midpoint minus 1% for sells or plus 1% for buys. Keep lots within 1-10. A ladder steps each next lot's limit away from the first by ladder_step_bps.`;

const headers = () => ({
  "content-type": "application/json",
  authorization: `Bearer ${process.env.NEARAI_API_KEY ?? ""}`,
});

function recoverPersonalSign(message: string, signatureHex: string): string {
  const sig = hexToBytes(signatureHex.replace(/^0x/, ""));
  const msg = new TextEncoder().encode(message);
  const prefix = new TextEncoder().encode(`\x19Ethereum Signed Message:\n${msg.length}`);
  const digest = keccak_256(new Uint8Array([...prefix, ...msg]));
  const v = sig[64] >= 27 ? sig[64] - 27 : sig[64];
  const pub = secp256k1.Signature.fromCompact(sig.slice(0, 64)).addRecoveryBit(v).recoverPublicKey(digest).toRawBytes(false);
  return `0x${bytesToHex(keccak_256(pub.slice(1)).slice(12))}`;
}

function jwtPayload(token: string): Record<string, unknown> {
  const part = token.split(".")[1] ?? "";
  return JSON.parse(Buffer.from(part.replace(/-/g, "+").replace(/_/g, "/"), "base64").toString("utf8"));
}

function validPlan(raw: unknown): LotPlan | null {
  if (!raw || typeof raw !== "object") return null;
  const p = raw as Record<string, unknown>;
  const int = (v: unknown, lo: number, hi: number, d: number) => {
    const n = Math.round(Number(v));
    return Number.isFinite(n) ? Math.min(hi, Math.max(lo, n)) : d;
  };
  if (p.side !== "sell" && p.side !== "buy") return null;
  const price = String(p.limit_price ?? "");
  if (!/^\d+(\.\d+)?$/.test(price)) return null;
  const lots = int(p.lots, 1, 10, 1);
  return {
    side: p.side,
    lots,
    limit_price: price,
    ladder_step_bps: int(p.ladder_step_bps, 0, 100, 0),
    min_lots: int(p.min_lots, 1, lots, 1),
    ttl_minutes: int(p.ttl_minutes, 10, 1440, 60),
    rationale: String(p.rationale ?? "").slice(0, 280),
  };
}

export async function plan(mandate: string, context: { mid: number; lotLabel: string; market: string }) {
  const checks: BrokerCheck[] = [];
  const nonce = bytesToHex(randomBytes(32));

  // 1. Attest the model fresh, bound to our nonce.
  const report = (await fetch(
    `${BASE}/v1/attestation/report?model=${encodeURIComponent(BROKER_MODEL)}&signing_algo=ecdsa&nonce=${nonce}`,
    { headers: headers() },
  ).then((r) => (r.ok ? r.json() : null))) as {
    request_nonce?: string;
    model_attestations?: { signing_address?: string; intel_quote?: string; nvidia_payload?: string | object }[];
  } | null;
  const nodes = report?.model_attestations ?? [];
  const signers = nodes.map((n) => n.signing_address?.toLowerCase()).filter((a): a is string => Boolean(a));
  const node = nodes[0];
  let quoteBinds: boolean | null = null;
  if (node?.intel_quote) {
    const q = hexToBytes(node.intel_quote.replace(/^0x/, ""));
    const reportData = q.slice(568, 632);
    const addr = (node.signing_address ?? "").toLowerCase().replace(/^0x/, "");
    quoteBinds = bytesToHex(reportData.slice(32)) === nonce && bytesToHex(reportData.slice(0, 32)).includes(addr);
  }
  checks.push({
    id: "tdx",
    label: "Model runs in an Intel TDX enclave, bound to this request",
    ok: quoteBinds,
    detail:
      quoteBinds === null
        ? "no attestation report returned"
        : `TDX report data carries our fresh nonce and the signing address ${node?.signing_address}. The quote's Intel signature chain is not re-verified in this path.`,
  });

  let gpuOk: boolean | null = null;
  if (node?.nvidia_payload) {
    const nras = (await fetch(NRAS, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: typeof node.nvidia_payload === "string" ? node.nvidia_payload : JSON.stringify(node.nvidia_payload),
    })
      .then((r) => (r.ok ? r.json() : null))
      .catch(() => null)) as [unknown, unknown] | null;
    const token = Array.isArray(nras) && Array.isArray(nras[0]) ? String((nras[0] as unknown[])[1]) : null;
    gpuOk = token ? jwtPayload(token)["x-nvidia-overall-att-result"] === true : false;
  }
  checks.push({
    id: "gpu",
    label: "GPU confidential computing verified by NVIDIA",
    ok: gpuOk,
    detail: gpuOk === null ? "no GPU evidence returned" : `NVIDIA Remote Attestation Service overall result: ${gpuOk}`,
  });

  // 2. Ask the attested model. The exact request bytes are what get signed.
  const requestBody = JSON.stringify({
    model: BROKER_MODEL,
    stream: false,
    temperature: 0,
    messages: [
      { role: "system", content: SYSTEM },
      {
        role: "user",
        content: `Market ${context.market}, lot ${context.lotLabel}, reference midpoint ${context.mid.toFixed(4)}.\nMandate: ${mandate}`,
      },
    ],
  });
  const res = await fetch(`${BASE}/v1/chat/completions`, { method: "POST", headers: headers(), body: requestBody });
  const responseText = await res.text();
  if (!res.ok) return { ok: false as const, error: `model request failed (${res.status})`, checks };
  const completion = JSON.parse(responseText) as { id?: string; choices?: { message?: { content?: string } }[] };
  const content = completion.choices?.[0]?.message?.content ?? "";
  const json = /\{[\s\S]*\}/.exec(content)?.[0];
  const lotPlan = json ? validPlan(JSON.parse(json)) : null;

  // 3. Check the response signature against the attested signing key.
  const reqHash = bytesToHex(sha256(new TextEncoder().encode(requestBody)));
  const resHash = bytesToHex(sha256(new TextEncoder().encode(responseText)));
  let sigOk: boolean | null = null;
  let sigDetail = "no signature returned";
  if (completion.id) {
    for (let attempt = 0; attempt < 5 && sigOk === null; attempt++) {
      const sig = (await fetch(
        `${BASE}/v1/signature/${encodeURIComponent(completion.id)}?model=${encodeURIComponent(BROKER_MODEL)}&signing_algo=ecdsa`,
        { headers: headers() },
      ).then((r) => (r.ok ? r.json() : null))) as { text?: string; signature?: string } | null;
      if (!sig?.text || !sig.signature) {
        await new Promise((r) => setTimeout(r, 800));
        continue;
      }
      const parts = sig.text.split(":");
      const hashesMatch = parts.at(-2) === reqHash && parts.at(-1) === resHash;
      const signer = recoverPersonalSign(sig.text, sig.signature).toLowerCase();
      sigOk = hashesMatch && signers.includes(signer);
      sigDetail = `signed ${parts.length === 3 ? `by the model enclave for ${parts[0]}` : "by the gateway enclave"} over sha256(request) and sha256(response); signer ${signer}`;
    }
  }
  checks.push({ id: "signature", label: "This exact answer was signed by the attested key", ok: sigOk, detail: sigDetail });

  if (!lotPlan) return { ok: false as const, error: "the model did not return a usable plan", checks };
  return { ok: true as const, plan: lotPlan, model: BROKER_MODEL, checks };
}
