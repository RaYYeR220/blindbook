"use client";

import { type KeyboardEvent, useRef } from "react";
import { fmtPrice } from "@/lib/app/format";
import { accountUrl, MARKET, NET, NETWORK_LABEL } from "@/lib/app/net";
import type { RouteCurve } from "@/lib/app/types";
import { AppProvider, type Pane, useApp } from "./AppContext";
import { AttestationRail } from "./AttestationRail";
import { BatchTape } from "./BatchTape";
import { DemoPanel } from "./DemoPanel";
import { MyOrders } from "./MyOrders";
import { OrderTicket } from "./OrderTicket";
import { TcaPanel } from "./TcaPanel";
import { Ext } from "./ui";
import { WalletPanel } from "./WalletPanel";

export function TradingApp({ curve }: { curve: RouteCurve }) {
  return (
    <AppProvider>
      <Strip />
      <Tabs />
      <Grid curve={curve} />
    </AppProvider>
  );
}

/** One line of live, public pool state. */
function Strip() {
  const { pool, config, mid } = useApp();
  const cfg = config.data ?? pool.data?.config;
  const stats = pool.data?.stats;
  const crossed = stats?.markets.reduce((n, m) => n + m.crossed_lots, 0);
  const stale = Boolean(pool.error);
  return (
    <div className="strip" role="status" aria-live="off">
      <div className="strip-in">
        <span className="strip-net">
          <i className={stale ? "is-stale" : ""} aria-hidden="true" />
          {NETWORK_LABEL}
        </span>
        <span>
          <span className="strip-k">pool</span>{" "}
          <Ext href={accountUrl(NET.pool)}>
            <code>{NET.pool}</code>
          </Ext>
        </span>
        <span>
          <span className="strip-k">
            {MARKET.baseSymbol.replace(/^t/, "")}/{MARKET.quoteSymbol.replace(/^t/, "")} mid
          </span>{" "}
          <b className="num">{mid.data ? fmtPrice(mid.data.mid, 4) : "–"}</b>
        </span>
        <span>
          <span className="strip-k">next batch</span> <b className="num">{cfg ? `#${cfg.next_batch_id}` : "–"}</b>
        </span>
        <span>
          <span className="strip-k">waiting</span> <b className="num">{cfg ? cfg.pending : "–"}</b>
        </span>
        <span className="strip-wide">
          <span className="strip-k">batches</span> <b className="num">{stats ? stats.totals.batches : "–"}</b>
          <span className="strip-k"> · lots crossed</span> <b className="num">{crossed ?? "–"}</b>
        </span>
        {stale ? <span className="bad">pool read failed, retrying</span> : null}
      </div>
    </div>
  );
}

const PANES: { id: Pane; label: string }[] = [
  { id: "trade", label: "Trade" },
  { id: "tape", label: "Tape" },
  { id: "proof", label: "Attestation" },
];

/** Under 860px the tape and the attestation rail sit behind tabs. */
function Tabs() {
  const { pane, setPane } = useApp();
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const i = (PANES.findIndex((p) => p.id === pane) + step + PANES.length) % PANES.length;
    setPane(PANES[i].id);
    refs.current[i]?.focus();
  };
  return (
    <div className="tabs" role="tablist" aria-label="App sections" onKeyDown={onKey}>
      {PANES.map((p, i) => (
        <button
          key={p.id}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="tab"
          id={`tab-${p.id}`}
          aria-selected={pane === p.id}
          aria-controls={`pane-${p.id}`}
          tabIndex={pane === p.id ? 0 : -1}
          onClick={() => setPane(p.id)}
        >
          {p.label}
        </button>
      ))}
    </div>
  );
}

function Grid({ curve }: { curve: RouteCurve }) {
  const { pane } = useApp();
  return (
    <div className="grid" data-pane={pane}>
      <div className="g-trade" id="pane-trade">
        <div className="g-demo">
          <DemoPanel />
        </div>
        <aside className="g-ticket" aria-label="Wallet and order ticket">
          <WalletPanel />
          <OrderTicket />
        </aside>
        <div className="g-status">
          <MyOrders />
          <TcaPanel curve={curve} />
        </div>
      </div>
      <div className="g-tape" id="pane-tape">
        <BatchTape />
      </div>
      <aside className="g-proof" id="pane-proof" aria-label="Attestation">
        <AttestationRail />
      </aside>
    </div>
  );
}
