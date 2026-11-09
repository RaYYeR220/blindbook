"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/app/api";
import { nonceUsed, rememberCrank } from "@/lib/app/chain";
import { fmtAgo, fmtLeft, fmtPrice, lotPrice } from "@/lib/app/format";
import { LOT_UNITS, MARKET } from "@/lib/app/net";
import { fillQuote, type LotStatus, lotStatuses, signCancelTickets, type StoredOrder } from "@/lib/app/orders";
import { walletErrorMessage } from "@/lib/app/wallet";
import { useApp } from "./AppContext";
import { useNow } from "./hooks";
import { PanelHead, Tile, type TileKind, TxLink } from "./ui";

const TILE_OF: Record<LotStatus["state"], TileKind> = {
  pending: "sealed",
  resting: "sealed",
  crossed: "crossed",
  cancelled: "open",
  expired: "void",
  refused: "refused",
};

const isOpen = (s: LotStatus) => s.state === "resting" || s.state === "pending";

/** What one crossed lot settled at, from the batch's public record. */
function lotFill(s: LotStatus, side: "sell" | "buy"): number | null {
  const m = s.batch?.markets.find((x) => x.market === MARKET.id);
  const q = m ? fillQuote(m, side) : null;
  return q === null ? null : lotPrice(MARKET, q);
}

export function MyOrders() {
  const { wallet, orders, updateOrder, batches, pool, config, refreshChain, setFocus, setFill, balances } = useApp();
  const [used, setUsed] = useState<Record<string, boolean>>({});
  const [cancel, setCancel] = useState<{ key: string; done: number; total: number; error?: string } | null>(null);
  const [crank, setCrank] = useState<{ state: "running" | "done" | "error"; startedAt: number; tx?: string; batchId?: number; error?: string } | null>(null);
  const now = useNow(1000);
  const list = batches.data ?? [];
  const checkedExpired = useRef(new Set<string>());

  // ask the verifier which nonces are spent, for every lot not yet settled in our records
  const account = wallet?.accountId ?? null;
  useEffect(() => {
    if (!account) return;
    let alive = true;
    const check = async () => {
      const t = Date.now();
      const todo = orders.flatMap((o) =>
        o.lots
          .filter((l) => !o.fills?.[l.payload.nonce] && !o.cancel?.nonces.includes(l.payload.nonce))
          .filter((l) => Date.parse(o.deadline) > t || !checkedExpired.current.has(l.payload.nonce))
          .map((l) => ({ nonce: l.payload.nonce, expired: Date.parse(o.deadline) <= t })),
      );
      const results = await Promise.all(
        todo.map(async ({ nonce, expired }) => {
          const u = await nonceUsed(account, nonce).catch(() => null);
          if (u !== null && expired) checkedExpired.current.add(nonce);
          return [nonce, u] as const;
        }),
      );
      if (!alive) return;
      setUsed((prev) => {
        const next = { ...prev };
        for (const [n, u] of results) if (u !== null) next[n] = u;
        return next;
      });
    };
    void check();
    const id = window.setInterval(() => {
      if (document.visibilityState === "visible") void check();
    }, 10_000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [account, orders]);

  const views = useMemo(
    () => orders.map((o) => ({ order: o, lots: lotStatuses(o, used, list, now ?? 0) })),
    [orders, used, list, now],
  );

  // remember which batch each newly crossed lot settled in, and show the latest fill's TCA
  const announced = useRef(new Set<string>());
  useEffect(() => {
    for (const { order, lots } of views) {
      const fresh = lots.filter((s) => s.state === "crossed" && s.batch && !order.fills?.[s.lot.payload.nonce]);
      if (fresh.length === 0) continue;
      updateOrder(order.key, (o) => ({
        ...o,
        fills: { ...o.fills, ...Object.fromEntries(fresh.map((s) => [s.lot.payload.nonce, s.batch!.id])) },
      }));
      const s = fresh[0];
      const price = lotFill(s, order.side);
      const m = s.batch!.markets.find((x) => x.market === MARKET.id);
      const tag = `${order.key}:${s.batch!.id}`;
      if (price !== null && m && !announced.current.has(tag)) {
        announced.current.add(tag);
        setFill({
          side: order.side,
          lots: fresh.length,
          mid: lotPrice(MARKET, m.lot_mid_quote),
          price,
          limit: lotPrice(MARKET, s.lot.limitQuote),
          batchId: s.batch!.id,
          source: "order",
        });
        void balances.refresh();
      }
    }
  }, [views, updateOrder, setFill, balances]);

  const improvement = views.reduce((sum, { order, lots }) => {
    for (const s of lots) {
      if (s.state !== "crossed") continue;
      const f = lotFill(s, order.side);
      if (f === null) continue;
      const lim = lotPrice(MARKET, s.lot.limitQuote);
      sum += (order.side === "sell" ? f - lim : lim - f) * LOT_UNITS;
    }
    return sum;
  }, 0);
  const crossedCount = views.reduce((n, v) => n + v.lots.filter((s) => s.state === "crossed").length, 0);
  const openCount = views.reduce((n, v) => n + v.lots.filter(isOpen).length, 0);

  const doCancel = async (order: StoredOrder, open: LotStatus[]) => {
    if (!wallet) return;
    setCancel({ key: order.key, done: 0, total: open.length });
    try {
      const tickets = await signCancelTickets(
        open.map((s) => s.lot),
        (p) => wallet.sign(p),
        (done, total) => setCancel({ key: order.key, done, total }),
      );
      const r = await api.cancel(tickets);
      if (!r.ok) throw new Error(r.error ? `the verifier refused the tickets: ${r.error.slice(0, 160)}` : "the relay could not send the tickets");
      updateOrder(order.key, (o) => ({
        ...o,
        cancel: { tx: r.tx, at: Date.now(), nonces: [...(o.cancel?.nonces ?? []), ...open.map((s) => s.lot.payload.nonce)] },
      }));
      setCancel(null);
    } catch (e) {
      setCancel({ key: order.key, done: 0, total: open.length, error: walletErrorMessage(e) });
    }
  };

  const cfg = config.data ?? pool.data?.config;
  const pending = cfg?.pending ?? 0;
  const nextBatch = cfg?.next_batch_id;
  const inFlight = cfg?.in_flight ?? null;
  const runBatch = async () => {
    setCrank({ state: "running", startedAt: Date.now() });
    const r = await api.crank();
    if (r.ok && r.tx && r.batchId !== undefined && r.cranker) {
      rememberCrank(r.batchId, r.tx, r.cranker);
      setFocus({ batchId: String(r.batchId), tx: r.tx, source: "crank" });
      setCrank({ state: "done", startedAt: 0, tx: r.tx, batchId: r.batchId });
    } else {
      setCrank({ state: "error", startedAt: 0, tx: r.tx, error: r.error ?? "the batch did not run" });
    }
    refreshChain();
  };
  const cranking = crank?.state === "running";

  return (
    <section className="panel orders" aria-labelledby="orders-h">
      <PanelHead
        id="orders-h"
        title="Your sealed lots"
        meta={
          <span className="num">
            {crossedCount > 0 ? `+${fmtPrice(improvement, 4)} ${MARKET.quoteSymbol} returned · ` : ""}
            {openCount} open · {crossedCount} crossed
          </span>
        }
      />
      <p className="pin">
        <span className="pin-k">Worst case: your own limit.</span> Each lot is a signed <code>token_diff</code> at its limit.
        The verifier applies it exactly or not at all, whatever the matcher does.
      </p>

      <div className="batchbar">
        <span className="num">
          {nextBatch !== undefined ? `next batch #${nextBatch}` : "next batch –"} ·{" "}
          {inFlight ? `batch #${inFlight.batch_id} in the enclave` : `${pending} sealed ${pending === 1 ? "envelope" : "envelopes"} waiting`}
        </span>
        <button
          type="button"
          className="btn btn-line btn-sm"
          disabled={pending === 0 || cranking || inFlight !== null}
          onClick={runBatch}
          title={pending === 0 ? "Nothing is waiting for a batch" : undefined}
        >
          {cranking ? `Running… ${now && crank ? Math.floor((now - crank.startedAt) / 1000) : 0} s` : "Run batch now"}
        </button>
        <span className="batchbar-h">Anyone can crank. The relay pays the gas and rests house lots on both sides first.</span>
        <div className="batchbar-out" aria-live="polite">
          {crank?.state === "done" && crank.tx ? (
            <span>
              Batch <span className="num">#{crank.batchId}</span> ran <TxLink hash={crank.tx} />
            </span>
          ) : null}
          {crank?.state === "error" ? <span className="is-bad">{crank.error}</span> : null}
        </div>
      </div>

      {!wallet ? (
        <div className="empty">
          <span className="empty-art" aria-hidden="true">
            <Tile kind="void" />
            <Tile kind="sealed" />
            <Tile kind="void" />
          </span>
          <p>Connect a wallet to see the lots you seal from this browser. The demo above needs no wallet.</p>
        </div>
      ) : views.length === 0 ? (
        <div className="empty">
          <span className="empty-art" aria-hidden="true">
            <Tile kind="void" />
            <Tile kind="void" />
            <Tile kind="void" />
          </span>
          <p>Nothing sealed from this browser yet. Seal a block from the ticket.</p>
        </div>
      ) : (
        <ul className="olist">
          {views.map(({ order, lots }) => {
            const open = lots.filter(isOpen);
            const left = now ? Date.parse(order.deadline) - now : null;
            const busy = cancel?.key === order.key && !cancel.error;
            return (
              <li key={order.key} className="ocard">
                <div className="ocard-h">
                  <span className={`side is-${order.side}`}>{order.side === "sell" ? "Sell" : "Buy"}</span>
                  <span className="num">
                    {order.lots.length} × {LOT_UNITS} {MARKET.baseSymbol}
                  </span>
                  <span className="ocard-m num">
                    {order.orderId ? `order #${order.orderId}` : "order"} · {now ? fmtAgo(now - order.createdAt) : ""}
                    {open.length > 0 && left !== null ? ` · ${fmtLeft(left)} left` : ""}
                  </span>
                  <TxLink hash={order.submitTx} />
                  {open.length > 0 ? (
                    <button type="button" className="linkbtn" disabled={busy || !wallet} onClick={() => doCancel(order, open)}>
                      {busy && cancel ? `Signing ${cancel.done}/${cancel.total}…` : `Cancel ${open.length} open`}
                    </button>
                  ) : null}
                </div>
                {cancel?.key === order.key && cancel.error ? (
                  <p className="hint is-bad" role="alert">
                    {cancel.error}
                  </p>
                ) : null}
                <ol className="lots-l">
                  {lots.map((s) => (
                    <LotRow key={s.lot.payload.nonce} s={s} side={order.side} cancelTx={order.cancel?.tx} />
                  ))}
                </ol>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function LotRow({ s, side, cancelTx }: { s: LotStatus; side: "sell" | "buy"; cancelTx?: string }) {
  const limit = lotPrice(MARKET, s.lot.limitQuote);
  const fill = s.state === "crossed" ? lotFill(s, side) : null;
  let label: string;
  switch (s.state) {
    case "pending":
      label = "sealed · waiting for the next batch";
      break;
    case "resting":
      label = "sealed · resting";
      break;
    case "crossed":
      label = s.batch ? `crossed at mid · batch #${s.batch.id}` : "crossed at mid";
      break;
    case "cancelled":
      label = "cancelled by your ticket";
      break;
    case "expired":
      label = "expired unfilled";
      break;
    case "refused":
      label = `refused: ${s.reason ?? "invalid"}`;
      break;
  }
  return (
    <li className={`lot is-${s.state}`}>
      <Tile kind={TILE_OF[s.state]} small />
      <span className="lot-i num">lot {s.lot.index + 1}</span>
      <span className="lot-p num" title="your limit">
        {fmtPrice(limit, 4)}
      </span>
      <span className="lot-s">{label}</span>
      <span className="lot-x num">
        {fill !== null ? (
          <>
            {fmtPrice(fill, 4)} {side === "sell" ? "≥" : "≤"} limit ✓
          </>
        ) : s.state === "cancelled" && cancelTx ? (
          <TxLink hash={cancelTx} />
        ) : null}
      </span>
    </li>
  );
}

