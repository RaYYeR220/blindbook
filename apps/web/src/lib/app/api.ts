import type { MultiPayload, Side } from "@blindbook/sdk";
import type {
  BalancesResponse,
  BatchView,
  BrokerInfo,
  BrokerResponse,
  CrankResponse,
  DemoResponse,
  FaucetResponse,
  PoolResponse,
  RelayCancelResponse,
  RelaySubmitResponse,
  RouteCostResponse,
  VerifyResponse,
} from "./types";

/** A failed call to one of the app's own routes, with the route's own message. */
export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function call<T>(path: string, init?: { method?: "GET" | "POST"; body?: unknown; signal?: AbortSignal }): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method: init?.method ?? "GET",
      headers: init?.body === undefined ? undefined : { "content-type": "application/json" },
      body: init?.body === undefined ? undefined : JSON.stringify(init.body),
      cache: "no-store",
      signal: init?.signal,
    });
  } catch (e) {
    if ((e as Error).name === "AbortError") throw e;
    throw new ApiError("the app server did not answer; check your connection", 0);
  }
  const text = await res.text();
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    throw new ApiError(`unexpected response from ${path.split("?")[0]} (HTTP ${res.status})`, res.status);
  }
  if (!res.ok) {
    const b = body as { error?: string; reason?: string };
    throw new ApiError(b.error ?? b.reason ?? `HTTP ${res.status}`, res.status);
  }
  return body as T;
}

const q = (params: Record<string, string | number | undefined | null>) =>
  new URLSearchParams(
    Object.entries(params)
      .filter((e): e is [string, string | number] => e[1] !== undefined && e[1] !== null && e[1] !== "")
      .map(([k, v]) => [k, String(v)]),
  ).toString();

export const api = {
  pool: () => call<PoolResponse>("/api/pool"),
  batches: (limit = 12) => call<{ batches: BatchView[] }>(`/api/batches?${q({ limit })}`).then((r) => r.batches),
  verify: (batch: string, tx?: string | null, sender?: string | null) =>
    call<VerifyResponse>(`/api/verify?${q({ batch, tx, sender })}`),
  routeCost: (side: Side, near: number, signal?: AbortSignal) =>
    call<RouteCostResponse>(`/api/route-cost?${q({ side, near })}`, { signal }),
  balances: (account: string, key?: string | null) =>
    call<BalancesResponse>(`/api/balances?${q({ account, key })}`),
  faucetInfo: () => call<{ available: boolean }>("/api/faucet"),
  faucet: async (account: string): Promise<FaucetResponse> => {
    try {
      return await call<FaucetResponse>("/api/faucet", { method: "POST", body: { account } });
    } catch (e) {
      if (e instanceof ApiError) return { ok: false, error: e.message };
      throw e;
    }
  },
  demoInfo: () => call<{ available: boolean }>("/api/demo"),
  /** The demo answers 429 while a batch is cooling down; both outcomes come back as a result. */
  demo: async (side: Side): Promise<{ status: number; result: DemoResponse }> => {
    const res = await fetch("/api/demo", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ side }),
      cache: "no-store",
    });
    const result = (await res.json().catch(() => ({ ok: false, side, reason: `HTTP ${res.status}` }))) as DemoResponse;
    return { status: res.status, result };
  },
  submit: (market: string, sealedB64: string) =>
    call<RelaySubmitResponse>("/api/relay/submit", { method: "POST", body: { market, sealed_b64: sealedB64 } }),
  cancel: (tickets: MultiPayload[]) => call<RelayCancelResponse>("/api/relay/cancel", { method: "POST", body: { tickets } }),
  crank: async (): Promise<CrankResponse> => {
    try {
      return await call<CrankResponse>("/api/crank", { method: "POST" });
    } catch (e) {
      if (e instanceof ApiError) return { ok: false, error: e.message };
      throw e;
    }
  },
  brokerInfo: () => call<BrokerInfo>("/api/broker"),
  broker: async (body: { mandate: string; mid: number; lot: string; market: string }): Promise<BrokerResponse> => {
    const res = await fetch("/api/broker", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      cache: "no-store",
    }).catch(() => null);
    if (!res) return { ok: false, error: "the app server did not answer" };
    return (await res.json().catch(() => ({ ok: false, error: `HTTP ${res.status}` }))) as BrokerResponse;
  },
};
