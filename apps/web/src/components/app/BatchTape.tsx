"use client";

import { fmtAgo, fmtPrice, lotPrice, nsToDate } from "@/lib/app/format";
import { MARKET } from "@/lib/app/net";
import type { BatchStatus, BatchView } from "@/lib/app/types";
import { useApp } from "./AppContext";
import { useNow } from "./hooks";
import { PanelHead } from "./ui";

const STATUS_LABEL: Record<BatchStatus, string> = {
  requested: "in enclave",
  expired: "expired",
  failed: "failed",
  rejected_signature: "bad signature",
  invalid_result: "invalid result",
  unpinned: "unpinned",
  completed: "no cross",
  settling: "settling",
  settled: "settled",
  partially_settled: "part settled",
  settlement_failed: "settle failed",
};

/** The pool's public post-trade prints. Resting orders are never on it, for anyone. */
export function BatchTape() {
  const { batches, focus, setFocus, setPane } = useApp();
  const now = useNow(5000);
  const rows = batches.data ?? [];
  const shown = focus?.batchId ?? latestCrossed(rows)?.id ?? null;

  return (
    <section className="panel tape" aria-labelledby="tape-h">
      <PanelHead
        id="tape-h"
        title="Batch tape"
        meta={
          <span className="live num" aria-live="off">
            <i aria-hidden="true" className={batches.error ? "is-stale" : ""} />
            {batches.at && now ? `updated ${fmtAgo(now - batches.at)}` : batches.loading ? "loading" : "–"} · every 10 s
          </span>
        }
      />
      <p className="muted">Public prints only: what settlement reveals anyway. Pick a batch number to re-check it.</p>
      {batches.error && rows.length === 0 ? (
        <p className="hint is-bad" role="alert">
          The tape could not load: {batches.error}
        </p>
      ) : null}
      <div className="tape-scroll" role="region" aria-labelledby="tape-h" tabIndex={0}>
        <table className="tbl">
          <thead>
            <tr>
              <th scope="col">Batch</th>
              <th scope="col">Status</th>
              <th scope="col" className="r">
                Lots
              </th>
              <th scope="col" className="r">
                Mid
              </th>
              <th scope="col" className="r">
                Sub
              </th>
              <th scope="col">Quote digests</th>
              <th scope="col" className="r">
                Settled
              </th>
              <th scope="col" className="r">
                When
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && batches.loading
              ? Array.from({ length: 5 }, (_, i) => (
                  <tr key={i} className="is-sk" aria-hidden="true">
                    <td colSpan={8}>
                      <span className="sk" />
                    </td>
                  </tr>
                ))
              : null}
            {rows.length === 0 && !batches.loading && !batches.error ? (
              <tr>
                <td colSpan={8} className="tbl-empty">
                  No batches yet.
                </td>
              </tr>
            ) : null}
            {rows.map((b) => {
              const m = b.markets.find((x) => x.market === MARKET.id) ?? b.markets[0];
              const lots = b.markets.reduce((n, x) => n + x.matched_lots, 0);
              const settledAll = b.settled.length > 0 && b.settled.every(Boolean);
              const when = b.finished_at ?? b.requested_at;
              const isOn = shown === b.id;
              return (
                <tr key={b.id} className={`${isOn ? "is-on" : ""}${lots > 0 ? " is-x" : ""}`} aria-current={isOn ? "true" : undefined}>
                  <th scope="row">
                    <button
                      type="button"
                      className="rowbtn num"
                      onClick={() => {
                        setFocus({ batchId: b.id, source: "picked" });
                        setPane("proof");
                      }}
                      aria-label={`Check batch ${b.id}`}
                    >
                      #{b.id}
                    </button>
                  </th>
                  <td>
                    <span className={`st is-${b.status}`}>{STATUS_LABEL[b.status] ?? b.status}</span>
                  </td>
                  <td className="r num">{lots}</td>
                  <td className="r num">{m?.mid ? fmtPrice(lotPrice(MARKET, m.lot_mid_quote), 4) : "–"}</td>
                  <td className="r num">{b.sub_batches}</td>
                  <td className="dg num">
                    {m && m.quotes.length > 0
                      ? m.quotes.map((q) => (
                          <code key={q} title={q}>
                            {q.slice(0, 6)}
                          </code>
                        ))
                      : "–"}
                  </td>
                  <td className="r">
                    {lots > 0 ? (
                      settledAll ? (
                        <span className="ok" aria-label="settled">
                          ✓
                        </span>
                      ) : (
                        <span className="bad">{b.settled.filter(Boolean).length}/{b.sub_batches}</span>
                      )
                    ) : (
                      <span className="muted">–</span>
                    )}
                  </td>
                  <td className="r num muted">{now ? fmtAgo(now - nsToDate(when).getTime()) : ""}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** The newest batch that crossed and fully settled: the one worth checking by default. */
export function latestCrossed(rows: BatchView[]): BatchView | undefined {
  return rows.find((b) => b.status === "settled" && b.markets.some((m) => m.matched_lots > 0));
}
