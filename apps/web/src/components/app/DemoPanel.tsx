"use client";

import type { Side } from "@blindbook/sdk";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/app/api";
import { getBatch, poolConfig, rememberCrank } from "@/lib/app/chain";
import { fmtPrice, fmtSignedBps, lotPrice } from "@/lib/app/format";
import { LOT_LABEL, MARKET } from "@/lib/app/net";
import type { BatchStatus, DemoResponse, PoolConfigView } from "@/lib/app/types";
import { type Verification, verifyBatch, CHECK_SHORT } from "@/lib/app/verify";
import { useApp } from "./AppContext";
import { DemoScene } from "./DemoScene";
import { useNow } from "./hooks";
import { CheckList, PanelHead, TxLink } from "./ui";

const STEPS = [
  { label: "House rests the other side", note: `one ${LOT_LABEL} lot, 0.6% through the mid` },
  { label: "Your lot sealed", note: "signed at its limit, encrypted to the enclave key" },
  { label: "Batch cranked", note: "run_batch hands the ciphertexts to OutLayer" },
  { label: "Enclave crossed at the mid", note: "priced from two signed 1Click quotes" },
  { label: "Settled", note: "both legs in one execute_intents call" },
] as const;

/** The server allows one demo batch every 20 s. */
const COOLDOWN_MS = 20_000;
const AFTER_CRANK: BatchStatus[] = ["settling", "settled", "partially_settled", "settlement_failed"];

type Phase =
  | { kind: "idle" }
  | { kind: "running"; side: Side; startedAt: number; done: number }
  | { kind: "done"; side: Side; result: DemoResponse & { batch: NonNullable<DemoResponse["batch"]> }; took: number }
  | { kind: "error"; side: Side; message: string; retryAt: number | null; result?: DemoResponse };

const sideVerb = (s: Side) => (s === "sell" ? "Sold" : "Bought");

export function DemoPanel() {
  const { setFocus, setFill, refreshChain } = useApp();
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [verification, setVerification] = useState<{ v: Verification | null; error: string | null } | null>(null);
  const base = useRef<PoolConfigView | null>(null);
  const now = useNow(500, phase.kind === "running" || phase.kind === "error");
  const resultRef = useRef<HTMLDivElement>(null);

  // watch the chain while the server runs the demo, so each step lights when it lands
  const running = phase.kind === "running";
  useEffect(() => {
    if (!running) return;
    let alive = true;
    const tick = async () => {
      const b = base.current;
      const cfg = await poolConfig().catch(() => null);
      if (!alive || !cfg || !b) return;
      const cranked = cfg.next_batch_id > b.next_batch_id;
      let d = cranked ? 3 : cfg.pending >= b.pending + 2 ? 2 : cfg.pending >= b.pending + 1 ? 1 : 0;
      if (cranked) {
        const rec = await getBatch(b.next_batch_id).catch(() => null);
        if (rec && AFTER_CRANK.includes(rec.status) && rec.markets.some((m) => m.matched_lots > 0)) d = 4;
        if (rec?.status === "settled") d = 5;
      }
      if (alive) setPhase((p) => (p.kind === "running" && d > p.done ? { ...p, done: d } : p));
    };
    const id = window.setInterval(tick, 1500);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [running]);

  const run = useCallback(
    async (side: Side) => {
      setVerification(null);
      base.current = null;
      const startedAt = Date.now();
      setPhase({ kind: "running", side, startedAt, done: 0 });
      // the pool's state before the demo starts, so each step is read as a change from it
      base.current = await Promise.race([
        poolConfig().catch(() => null),
        new Promise<null>((r) => window.setTimeout(() => r(null), 2500)),
      ]);
      const { status, result } = await api
        .demo(side)
        .catch(() => ({ status: 0, result: { ok: false, side, reason: "the app server did not answer" } as DemoResponse }));
      refreshChain();
      if (status === 429) {
        setPhase({
          kind: "error",
          side,
          message: "A demo batch ran moments ago. The pool takes one every 20 seconds.",
          retryAt: Date.now() + COOLDOWN_MS,
        });
        return;
      }
      if (!result.ok || !result.batch || !result.txs) {
        setPhase({ kind: "error", side, message: result.reason ?? "The demo did not settle.", retryAt: null, result });
        return;
      }
      const batch = result.batch;
      rememberCrank(batch.id, result.txs.crank, batch.cranker);
      setPhase({ kind: "done", side: result.side, result: { ...result, batch }, took: Date.now() - startedAt });
      setFocus({ batchId: batch.id, tx: result.txs.crank, source: "demo" });
      const m = batch.markets.find((x) => x.market === MARKET.id);
      if (m && result.effective && result.limit) {
        setFill({
          side: result.side,
          lots: 1,
          mid: lotPrice(MARKET, m.lot_mid_quote),
          price: Number(result.effective),
          limit: Number(result.limit),
          batchId: batch.id,
          source: "demo",
        });
      }
      verifyBatch(batch, result.txs.crank)
        .then((v) => setVerification({ v, error: null }))
        .catch((e: Error) => setVerification({ v: null, error: e.message }));
    },
    [refreshChain, setFill, setFocus],
  );

  // move focus to the result once it lands, unless the visitor has moved on to something else
  const panelRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (phase.kind !== "done" && phase.kind !== "error") return;
    const active = document.activeElement;
    if (!active || active === document.body || panelRef.current?.contains(active)) {
      resultRef.current?.focus({ preventScroll: true });
    }
  }, [phase.kind]);

  const cooling = phase.kind === "error" && phase.retryAt !== null && now !== null && now < phase.retryAt;
  const wait = cooling && phase.kind === "error" && phase.retryAt && now ? Math.ceil((phase.retryAt - now) / 1000) : 0;
  const busy = phase.kind === "running";
  const done = phase.kind === "running" ? phase.done : phase.kind === "done" ? 5 : 0;
  const elapsed = phase.kind === "running" && now ? Math.max(0, Math.floor((now - phase.startedAt) / 1000)) : 0;

  return (
    <section className="panel demo" aria-labelledby="demo-h" ref={panelRef}>
      <PanelHead id="demo-h" title="Try it with no wallet" meta={<span className="num">~25 s · {LOT_LABEL} · testnet</span>} />
      <p className="demo-lede">
        A server-held demo account seals one lot at a limit 0.6% through the mid; the house takes the other side. Watch it
        cross at the verified midpoint, then check the batch yourself.
      </p>
      <div className="demo-ctas">
        <button type="button" className="btn btn-sell" disabled={busy || cooling} onClick={() => run("sell")}>
          Sell 1 lot in the dark
        </button>
        <button type="button" className="btn btn-buy" disabled={busy || cooling} onClick={() => run("buy")}>
          Buy 1 lot in the dark
        </button>
        {cooling ? (
          <span className="demo-wait num" aria-live="polite">
            ready in {wait} s
          </span>
        ) : null}
      </div>

      {phase.kind !== "idle" ? (
        <div className="demo-run">
          <DemoScene done={done} failed={phase.kind === "error"} />
          <ol className="steps" aria-label="Demo progress">
            {STEPS.map((s, i) => {
              const state = i < done ? "done" : busy && i === done ? "now" : phase.kind === "error" && i === done ? "fail" : "todo";
              return (
                <li key={s.label} className={`step is-${state}`}>
                  <span className="step-i num" aria-hidden="true">
                    {state === "done" ? "✓" : i + 1}
                  </span>
                  <span className="step-l">
                    {s.label}
                    <span className="sr-only">
                      {state === "done" ? ": done" : state === "now" ? ": in progress" : state === "fail" ? ": stopped" : ""}
                    </span>
                  </span>
                  <span className="step-n">{s.note}</span>
                </li>
              );
            })}
          </ol>
          <p className="sr-only" aria-live="polite" aria-atomic="true">
            {busy ? (done > 0 ? `${STEPS[done - 1].label}.` : "Starting the demo.") : ""}
          </p>
          {busy ? (
            <p className="demo-clock num" aria-hidden="true">
              {elapsed} s · watching {MARKET.id} on chain
            </p>
          ) : null}
        </div>
      ) : null}

      <div ref={resultRef} tabIndex={-1} className="demo-out">
        {phase.kind === "error" ? (
          <div className="note is-warn" role="status">
            <p>{phase.message}</p>
            {phase.result?.txs ? (
              <p className="txs">
                <TxLink hash={phase.result.txs.house} label="house" />
                <TxLink hash={phase.result.txs.order} label="order" />
                <TxLink hash={phase.result.txs.crank} label="crank" />
              </p>
            ) : null}
          </div>
        ) : null}
        {phase.kind === "done" ? <DemoResult phase={phase} verification={verification} /> : null}
      </div>
    </section>
  );
}

function DemoResult({
  phase,
  verification,
}: {
  phase: Extract<Phase, { kind: "done" }>;
  verification: { v: Verification | null; error: string | null } | null;
}) {
  const { result, side } = phase;
  const m = result.batch.markets.find((x) => x.market === MARKET.id);
  const mid = m ? lotPrice(MARKET, m.lot_mid_quote) : (result.mid ?? NaN);
  const limit = Number(result.limit);
  const fill = Number(result.effective);
  const inside = side === "sell" ? fill >= limit : fill <= limit;
  const improvement = Math.abs(fill - limit);
  const switched = phase.side !== result.side;
  const passing = verification?.v?.checks.filter((c) => c.ok === true).length ?? 0;
  const total = verification?.v?.checks.length ?? 7;

  return (
    <article className="fill" aria-labelledby="fill-h">
      <header className="fill-h">
        <h3 id="fill-h">
          {sideVerb(side)} {LOT_LABEL} in the dark
        </h3>
        <span className="pill is-lime num">
          batch #{result.batch.id} · {result.batch.status}
        </span>
      </header>
      {switched ? <p className="fill-note">The demo account was short on one side, so it traded the other.</p> : null}
      <dl className="kv">
        <div>
          <dt>Midpoint</dt>
          <dd>
            <b className="num">{fmtPrice(mid, 4)}</b>
            <span>signed 1Click quotes, verified in the enclave</span>
          </dd>
        </div>
        <div>
          <dt>Your limit</dt>
          <dd>
            <b className="num">{fmtPrice(limit, 4)}</b>
            <span className="num">mid {fmtSignedBps(((limit - mid) / mid) * 1e4)}</span>
          </dd>
        </div>
        <div className="is-key">
          <dt>Your fill</dt>
          <dd>
            <b className="num">{fmtPrice(fill, 4)}</b>
            <span className={inside ? "ok" : "bad"}>
              {side === "sell" ? "≥" : "≤"} your limit {inside ? "✓" : "✗"}
            </span>
          </dd>
        </div>
        <div>
          <dt>Returned to you</dt>
          <dd>
            <b className="num">+{fmtPrice(improvement, 4)}</b>
            <span className="num">
              {MARKET.quoteSymbol} past your limit · fill at mid {fmtSignedBps(((fill - mid) / mid) * 1e4)}
            </span>
          </dd>
        </div>
      </dl>
      <p className="txs">
        <TxLink hash={result.txs!.house} label="house order" />
        <TxLink hash={result.txs!.order} label="your order" />
        <TxLink hash={result.txs!.crank} label="crank + settle" />
      </p>
      <div className="fill-checks">
        <p className="fill-ck-h" aria-live="polite">
          {verification?.v
            ? `${passing} of ${total} checks pass, re-run from public sources`
            : verification?.error
              ? `Verification failed: ${verification.error}`
              : `Verifying batch #${result.batch.id} from public sources…`}
        </p>
        <CheckList checks={verification?.v?.checks ?? []} labels={CHECK_SHORT} stagger pending={verification ? 0 : 7} />
      </div>
    </article>
  );
}
