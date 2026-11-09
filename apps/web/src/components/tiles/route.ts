import { eOut, fit, hash, rrect, seg } from "./draw";
import { TILE } from "./palette";

/*
  "The route vs the book": every row is the same block of identical tiles,
  and each basis point the public route costs peels off it as one small white
  tile. Near the block they still sit in its grid; further out they splay,
  shrink and fade. Rows with no route draw an empty block; Blindbook's row
  keeps its block whole and lime-edged.
*/

export type RouteKind = "route" | "none" | "book";

export interface RouteRowInput {
  kind: RouteKind;
  /** whole basis points lost (0 for "none") */
  bps: number;
  art: HTMLElement;
}

interface Bp {
  x: number;
  y: number;
  col: number;
  u: number;
  i: number;
  sz: number;
  a: number;
}

interface RowGeo {
  kind: RouteKind;
  x: number;
  cy: number;
  ri: number;
  tiles: Bp[];
}

interface Params {
  n: number;
  tw: number;
  th: number;
  pitch: number;
  s: number;
  sp: number;
  R: number;
  gap: number;
  spread: number;
  jit: number;
  splay: number;
  stripW: number;
}

export interface RouteHandle {
  setRows(rows: RouteRowInput[]): void;
  layout(): void;
  /** play the peel from the start (first time on screen, or after a side switch) */
  replay(): void;
  draw(now: number): void;
  reset(): void;
}

export function createRoute(box: HTMLElement, cv: HTMLCanvasElement, reduced: boolean): RouteHandle {
  let sizes = new WeakMap<HTMLCanvasElement, { w: number; h: number }>();
  let inputs: RouteRowInput[] = [];
  let geo: { P: Params; rows: RowGeo[]; w: number } | null = null;
  let peelStart = -1; // -1: not played yet, draw the settled state
  let idleT0 = performance.now();

  function layout() {
    const b = box.getBoundingClientRect();
    const small = b.width < 640;
    const base = small
      ? { n: 7, tw: 16, th: 11, pitch: 20, s: 5, sp: 6.5, R: 5, gap: 6, spread: 0.36, jit: 4, splay: 0.25 }
      : { n: 14, tw: 20, th: 13, pitch: 25, s: 6, sp: 8.5, R: 3, gap: 8, spread: 0.6, jit: 7, splay: 0.55 };
    const P: Params = { ...base, stripW: base.n * base.pitch - (base.pitch - base.tw) };
    box.style.setProperty("--strip", `${P.stripW}px`);
    const rows = inputs.map((r, ri): RowGeo => {
      const a = r.art.getBoundingClientRect();
      const x = Math.round(a.left - b.left);
      const cy = Math.round(a.top - b.top + a.height / 2);
      const lostX = x + P.stripW + P.gap;
      const tiles: Bp[] = [];
      for (let i = 0; i < r.bps; i++) {
        const col = Math.floor(i / P.R);
        const row = i % P.R;
        const u = Math.min(1, col / 34);
        const tx = lostX + col * P.sp + Math.pow(col, 1.6) * P.spread + (hash(i * 3.1 + ri) - 0.5) * u * P.sp * 1.2;
        const jy = (hash(i * 7.3 + ri * 13.1) - 0.5) * u * 2 * P.jit;
        const ty = cy + (row - (P.R - 1) / 2) * P.sp * (1 + P.splay * u) + jy;
        tiles.push({ x: tx, y: ty, col, u, i, sz: P.s * (1 - 0.32 * u), a: 0.95 - 0.55 * u });
      }
      return { kind: r.kind, x, cy, ri, tiles };
    });
    geo = { P, rows, w: b.width };
  }

  function draw(now: number) {
    if (!geo) return;
    const { c, w, h } = fit(cv, sizes);
    c.clearRect(0, 0, w, h);
    const { P, rows } = geo;
    const t = peelStart < 0 ? 1e9 : now - peelStart;
    const idle = reduced || peelStart < 0 ? -1 : (now - idleT0) % 8000;
    const wob = !reduced && peelStart >= 0;
    for (const r of rows) {
      // the block: identical tiles, every row the same length
      for (let i = 0; i < P.n; i++) {
        const x = r.x + i * P.pitch + 0.5;
        const y = r.cy - Math.round(P.th / 2) + 0.5;
        rrect(c, x, y, P.tw - 1, P.th - 1, 2);
        if (r.kind !== "none") {
          c.fillStyle = TILE;
          c.fill();
        }
        c.strokeStyle =
          r.kind === "none" ? "rgba(255,255,255,.18)" : r.kind === "book" ? "rgba(212,242,74,.8)" : "rgba(255,255,255,.12)";
        c.lineWidth = 1;
        c.stroke();
      }
      if (r.kind === "none") continue;
      // the basis points that peel off to the route
      const startX = r.x + P.stripW - P.s;
      const start0 = r.ri * 80;
      for (const tl of r.tiles) {
        const k = eOut(seg(t, start0 + tl.col * 14, start0 + tl.col * 14 + 900));
        if (k <= 0) continue;
        let x = startX + (tl.x - startX) * k;
        let y = r.cy + (tl.y - r.cy) * k;
        if (wob && tl.u > 0) {
          x += Math.sin(now / 1700 + tl.i * 1.7) * 2.4 * tl.u;
          y += Math.cos(now / 2100 + tl.i) * 1.8 * tl.u;
        }
        let a = tl.a * Math.min(1, k * 1.6);
        if (idle >= 0 && idle < 2600) {
          const wave = r.x + P.stripW + (idle / 2600) * (w - r.x - P.stripW + 80) - 40;
          a = Math.min(1, a + 0.45 * Math.exp(-Math.pow((tl.x - wave) / 34, 2)));
        }
        c.globalAlpha = a;
        rrect(c, x, y - tl.sz / 2, tl.sz, tl.sz, 1.5);
        c.fillStyle = "#ffffff";
        c.fill();
        c.globalAlpha = 1;
      }
    }
  }

  return {
    setRows(rows) {
      inputs = rows;
      layout();
    },
    layout,
    replay() {
      if (reduced) return;
      peelStart = performance.now();
      idleT0 = peelStart + 2400;
    },
    draw,
    reset() {
      sizes = new WeakMap();
    },
  };
}
