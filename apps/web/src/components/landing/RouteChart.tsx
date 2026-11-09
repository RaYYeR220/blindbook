"use client";

import {
  createContext,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  use,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createRoute, type RouteHandle, type RouteKind } from "@/components/tiles/route";
import { onRelayout, register } from "@/components/tiles/scheduler";

export type Side = "sell" | "buy";

/** One row of the chart, formatted on the server. */
export interface RouteRowView {
  key: string;
  kind: RouteKind;
  /** whole basis points lost to the route */
  bps: number;
  /** first row after the last quoted size */
  cliff: boolean;
  size: string;
  sizeNote: string;
  cost: string;
  costNote: string | null;
  /** the route's own refusal, shown in the row */
  quote: string | null;
  tip: string;
}

const SideContext = createContext<{ side: Side; setSide: (s: Side) => void }>({ side: "sell", setSide: () => {} });

export function RouteSideProvider({ children }: { children: ReactNode }) {
  const [side, setSide] = useState<Side>("sell");
  return <SideContext value={{ side, setSide }}>{children}</SideContext>;
}

const SIDES: { side: Side; label: string }[] = [
  { side: "sell", label: "Selling NEAR" },
  { side: "buy", label: "Buying NEAR" },
];

/** Selling / Buying switch: a radio group with arrow-key movement. */
export function SideToggle() {
  const { side, setSide } = use(SideContext);
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const i = (SIDES.findIndex((o) => o.side === side) + step + SIDES.length) % SIDES.length;
    setSide(SIDES[i].side);
    refs.current[i]?.focus();
  };
  return (
    <div className="bb-seg" role="radiogroup" aria-label="Side" onKeyDown={onKey}>
      {SIDES.map((o, i) => (
        <button
          key={o.side}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="radio"
          aria-checked={side === o.side}
          tabIndex={side === o.side ? 0 : -1}
          onClick={() => setSide(o.side)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** One row per measured size; the canvas over them draws the block and the basis points it loses. */
export function RouteChart({ rows }: { rows: Record<Side, RouteRowView[]> }) {
  const { side } = use(SideContext);
  const boxRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const arts = useRef<(HTMLDivElement | null)[]>([]);
  const engine = useRef<RouteHandle | null>(null);
  const played = useRef(false);
  const view = rows[side];

  const inputs = () =>
    view.map((r, i) => ({ kind: r.kind, bps: r.bps, art: arts.current[i]! })).filter((r) => r.art);

  // mount: build the engine, draw the settled state, peel once on first sight
  useLayoutEffect(() => {
    const box = boxRef.current;
    const cv = canvasRef.current;
    if (!box || !cv) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const e = createRoute(box, cv, reduced);
    engine.current = e;
    e.setRows(inputs());
    e.draw(performance.now());
    const offRelayout = onRelayout(() => {
      e.reset();
      e.layout();
      e.draw(performance.now());
    });
    const unregister = register({
      el: box,
      frame: (now) => e.draw(now),
      onVisible: () => {
        if (played.current) return;
        played.current = true;
        e.replay();
      },
    });
    return () => {
      offRelayout();
      unregister();
      engine.current = null;
    };
    // the engine is built once; side changes are handled below
  }, []);

  // side switch: new rows, then replay the peel
  const first = useRef(true);
  useLayoutEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const e = engine.current;
    if (!e) return;
    e.setRows(inputs());
    played.current = true;
    e.replay();
    e.draw(performance.now());
  }, [side]);

  const onMove = (ev: PointerEvent<HTMLDivElement>) => {
    const tip = tipRef.current;
    const box = boxRef.current;
    if (!tip || !box) return;
    const row = (ev.target as HTMLElement).closest<HTMLElement>("[data-tip]");
    if (!row) {
      delete tip.dataset.on;
      return;
    }
    const b = box.getBoundingClientRect();
    tip.textContent = row.dataset.tip ?? "";
    tip.dataset.on = "1";
    const tw = tip.offsetWidth;
    let x = ev.clientX - b.left + 14;
    const y = ev.clientY - b.top + 16;
    if (x + tw > b.width - 4) x = ev.clientX - b.left - 14 - tw;
    tip.style.transform = `translate(${Math.max(0, x)}px, ${y}px)`;
  };
  const onLeave = () => {
    if (tipRef.current) delete tipRef.current.dataset.on;
  };

  return (
    <div className="route" ref={boxRef}>
      <div className="route-key" aria-hidden="true">
        <span className="key-block">
          <i />
          <i />
          <i />
          the block
        </span>
        <span className="key-bp">
          <i />1 basis point lost to the route
        </span>
      </div>
      <div className="route-rows" aria-hidden="true" onPointerMove={onMove} onPointerLeave={onLeave}>
        {view.map((r, i) => (
          <div
            key={`${side}-${r.key}`}
            className={["rrow", r.kind === "none" && "is-none", r.cliff && "is-cliff", r.kind === "book" && "is-book"]
              .filter(Boolean)
              .join(" ")}
            data-tip={r.tip}
          >
            <div className="rsize">
              <b>{r.size}</b>
              <span>{r.sizeNote}</span>
            </div>
            <div
              className="rart"
              ref={(el) => {
                arts.current[i] = el;
              }}
            >
              {r.quote ? <span className="rquote">“{r.quote}”</span> : null}
            </div>
            <div className="rcost">
              <b>{r.cost}</b>
              {r.costNote ? <span>{r.costNote}</span> : null}
            </div>
          </div>
        ))}
      </div>
      <canvas className="route-cv" ref={canvasRef} aria-hidden="true" />
      <div className="rtip" ref={tipRef} aria-hidden="true" />
    </div>
  );
}
