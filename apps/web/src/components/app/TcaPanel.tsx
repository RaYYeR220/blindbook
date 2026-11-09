"use client";

import type { Side } from "@blindbook/sdk";
import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/app/api";
import { bpsFromMid, fmtBps, fmtPrice, fmtUsd } from "@/lib/app/format";
import { LOT_UNITS, MARKET } from "@/lib/app/net";
import type { RouteCostResponse, RouteCurve } from "@/lib/app/types";
import { type Fill, useApp } from "./AppContext";
import { PanelHead } from "./ui";

const fmtDay = (iso: string) => new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });

/**
 * Transaction-cost analysis for one fill: what the same size would have lost
 * on the public 1Click route, measured live, against what the fill lost to
 * the mid. The chart is the measured route curve with the fill marked on it.
 */
export function TcaPanel({ curve }: { curve: RouteCurve }) {
  const { fill } = useApp();
  const [route, setRoute] = useState<{ key: string; data: RouteCostResponse | null; error: string | null } | null>(null);
  const size = fill ? fill.lots * LOT_UNITS : 0;
  const key = fill ? `${fill.batchId}:${fill.side}:${size}` : "";

  useEffect(() => {
    if (!fill) return;
    let alive = true;
    setRoute({ key, data: null, error: null });
    api
      .routeCost(fill.side, size)
      .then((data) => alive && setRoute({ key, data, error: null }))
      .catch((e: Error) => alive && setRoute({ key, data: null, error: e.message }));
    return () => {
      alive = false;
    };
  }, [key, fill, size]);

  const live = route?.key === key ? route.data : null;
  const ours = fill ? -bpsFromMid(fill.side, fill.price, fill.mid) : null;
  const routeBps = live?.route_cost_bps ?? null;
  const notional = fill ? size * fill.mid : 0;
  const kept = routeBps !== null && ours !== null ? ((routeBps - ours) / 1e4) * notional : null;
  const improvement = fill ? Math.abs(fill.price - fill.limit) * size : null;

  return (
    <section className="panel tca" aria-labelledby="tca-h">
      <PanelHead
        id="tca-h"
        title="Your fill vs the public route"
        meta={fill ? <span className="num">batch #{fill.batchId} · {fill.source === "demo" ? "demo" : "your lots"}</span> : null}
      />
      {fill ? (
        <dl className="tca-kv">
          <div>
            <dt>Your fill</dt>
            <dd>
              <b className="num">{fmtPrice(fill.price, 4)}</b>
              <span className="num">
                {ours !== null ? `${fmtBps(Math.abs(ours))} ${fill.side === "sell" ? "under" : "over"} mid` : ""}
              </span>
            </dd>
          </div>
          <div>
            <dt>
              Public route, {size} {MARKET.baseSymbol}
            </dt>
            <dd aria-busy={!live && !route?.error}>
              <b className="num">{routeBps !== null ? fmtBps(routeBps) : live ? "no route" : route?.error ? "–" : "…"}</b>
              <span>live 1Click dry quote</span>
            </dd>
          </div>
          <div className="is-key">
            <dt>Kept vs the route</dt>
            <dd>
              <b className="num">{kept !== null ? fmtUsd(kept) : "…"}</b>
              <span className="num">{routeBps !== null && ours !== null ? `${fmtBps(routeBps - ours)} on ${fmtUsd(notional)}` : ""}</span>
            </dd>
          </div>
          <div>
            <dt>Improvement returned</dt>
            <dd>
              <b className="num">{improvement !== null ? fmtUsd(improvement) : "–"}</b>
              <span>fill beyond your limit, paid back in the same call</span>
            </dd>
          </div>
        </dl>
      ) : (
        <p className="muted">
          Run the demo or cross a lot and its fill lands here, against what the public route charges for the same size.
        </p>
      )}
      <RouteChart curve={curve} side={fill?.side ?? "sell"} fill={fill} size={size} ours={ours} live={routeBps} />
      <p className="cite">
        Curve: 1Click dry quotes for NEAR/USDC on mainnet, measured {fmtDay(curve.measuredAt)} (results/ baseline). Live point:
        a fresh dry quote at your size. Testnet lots are priced off the same mainnet mid.
      </p>
    </section>
  );
}

/** The chart draws at its real width, so its 10px labels stay 10px on a phone. */
const DEFAULT_W = 600;
const H = 176;
const PAD = { l: 40, r: 14, t: 16, b: 28 };
const X_MIN = 1;
const X_MAX = 1_000_000;
const TICKS = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];
const tickLabel = (n: number) => (n >= 1_000_000 ? "1M" : n >= 1000 ? `${n / 1000}k` : String(n));

function RouteChart({
  curve,
  side,
  fill,
  size,
  ours,
  live,
}: {
  curve: RouteCurve;
  side: Side;
  fill: Fill | null;
  size: number;
  ours: number | null;
  live: number | null;
}) {
  const figRef = useRef<HTMLElement>(null);
  const [W, setW] = useState(DEFAULT_W);
  useEffect(() => {
    const el = figRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setW(Math.max(280, Math.round(entry.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const narrow = W < 460;
  const rows = curve.rows[side];
  const quoted = rows.filter((r): r is { near: number; bps: number } => r.bps !== null);
  const refused = rows.find((r) => r.bps === null && r.near > (quoted.at(-1)?.near ?? 0));
  const yMax = Math.max(40, ...quoted.map((r) => r.bps), live ?? 0) * 1.12;
  const x = (n: number) => PAD.l + ((Math.log10(n) - Math.log10(X_MIN)) / (Math.log10(X_MAX) - Math.log10(X_MIN))) * (W - PAD.l - PAD.r);
  const y = (b: number) => PAD.t + (1 - b / yMax) * (H - PAD.t - PAD.b);
  const yTicks = [0, 25, 50, 75, 100, 125, 150].filter((t) => t <= yMax);
  const path = quoted.map((r, i) => `${i ? "L" : "M"}${x(r.near).toFixed(1)},${y(r.bps).toFixed(1)}`).join(" ");
  const fx = fill ? x(Math.max(X_MIN, size)) : 0;

  const label = fill
    ? `Route cost by size. Your fill of ${size} ${MARKET.baseSymbol} lost ${ours?.toFixed(1)} bps to the mid; the route ${
        live !== null ? `would lose ${live.toFixed(1)} bps` : "has no live quote"
      } at that size.`
    : `Route cost by size, measured ${fmtDay(curve.measuredAt)}.`;

  return (
    <figure className="chart" ref={figRef}>
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={label} preserveAspectRatio="xMidYMid meet">
        {yTicks.map((t) => (
          <g key={t}>
            <line x1={PAD.l} x2={W - PAD.r} y1={y(t)} y2={y(t)} className="ch-grid" />
            <text x={PAD.l - 8} y={y(t) + 3.5} className="ch-t" textAnchor="end">
              {t}
            </text>
          </g>
        ))}
        {TICKS.filter((t, i) => !narrow || i % 2 === 0 || t === X_MAX).map((t) => (
          <text key={t} x={x(t)} y={H - 8} className="ch-t" textAnchor="middle">
            {tickLabel(t)}
          </text>
        ))}
        <text x={PAD.l} y={10} className="ch-t">
          bps lost vs mid
        </text>
        <text x={W - PAD.r} y={10} className="ch-t" textAnchor="end">
          size, {MARKET.baseSymbol.replace(/^t/, "")} (log)
        </text>

        <path d={path} className="ch-curve" />
        {quoted.map((r) => (
          <circle key={r.near} cx={x(r.near)} cy={y(r.bps)} r={2.2} className="ch-pt" />
        ))}
        {refused ? (
          <g>
            <line x1={x(refused.near)} x2={x(refused.near)} y1={PAD.t} y2={H - PAD.b} className="ch-wall" />
            <text x={x(refused.near) - 4} y={PAD.t + 10} className="ch-t" textAnchor="end">
              no route
            </text>
          </g>
        ) : null}

        {fill && live !== null ? (
          <g>
            <line x1={fx} x2={fx} y1={y(live)} y2={y(Math.max(0, ours ?? 0))} className="ch-gap" />
            <circle cx={fx} cy={y(live)} r={4} className="ch-live" />
            <text x={fx + 9} y={y(live) - 7} className="ch-t is-ink">
              route now {live.toFixed(1)}
            </text>
          </g>
        ) : null}
        {fill && ours !== null ? (
          <g>
            <rect x={fx - 9} y={y(Math.max(0, ours)) - 5.5} width={18} height={11} rx={2} className="ch-fill" />
            <text x={fx + 14} y={y(Math.max(0, ours)) + 3.5} className="ch-t is-lime">
              your fill {ours.toFixed(1)}
            </text>
          </g>
        ) : null}
      </svg>
    </figure>
  );
}
