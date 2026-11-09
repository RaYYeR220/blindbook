import {
  check,
  type Ctx,
  eIO,
  emptyTile,
  eOut,
  fit,
  hexFrom,
  type Leg,
  leg,
  mix,
  pointAt,
  rrect,
  sealed,
  seg,
  solid,
  strokePartial,
} from "./draw";
import { DEEP, type Faces, LIME, PX, PY, RAD, TH, TILE, TW, WHITE } from "./palette";

/*
  "How a block crosses": four small canvases replay one batch in sequence
  (seal, batch, cross, settle). One 10 s cycle, four windows of 2.5 s; the
  step whose window is running gets its number lit and its bar filling.
*/

export const CYCLE = 10_000;
export const WIN = 2_500;
/** Still key frames: mid-seal, batch bracket, crossed pairs, legs settling with the improvement in flight. */
const STILL = [1500, WIN + 1500, 2 * WIN + 2000, 3 * WIN + 1450];

export interface StepLabels {
  /** the seller's limit, shown on "your lot" before it is sealed */
  sellLimit: string;
  /** the mid the cross happens at, e.g. "mid 3.61498" */
  mid: string;
}

interface GTile {
  i: number;
  k: number;
  x: number;
  y: number;
}
interface GPair {
  s: GTile;
  b: GTile;
  xc: number;
  ls: Leg;
  lb: Leg;
}
interface Grid {
  cols: number;
  gw: number;
  x0: number;
  midY: number;
  tiles: GTile[];
  A: GPair;
  B: GPair;
  you: GTile;
  empties: Set<string>;
  w: number;
  h: number;
}

const key = (t: GTile) => `${t.i},${t.k}`;
const same = (a: GTile, b: GTile) => a.i === b.i && a.k === b.k;

function grid(w: number, h: number): Grid {
  const cols = Math.max(4, Math.floor((w - 16 + 12) / PX));
  const gw = cols * PX - 12;
  const x0 = Math.round((w - gw) / 2);
  const midY = Math.round(h / 2);
  const at = (i: number, k: number): GTile => ({ i, k, x: x0 + i * PX, y: midY + k * PY - TH / 2 });
  const tiles: GTile[] = [];
  for (const k of [-2, -1, 1, 2]) for (let i = 0; i < cols; i++) tiles.push(at(i, k));
  // pair A: diagonal across the channel; pair B: straight down one gap
  const pair = (s: GTile, b: GTile): GPair => {
    const xc = (s.x + b.x) / 2 + TW / 2;
    return {
      s,
      b,
      xc,
      ls: leg(s.x + TW / 2, s.y + TH / 2, xc, midY),
      lb: leg(b.x + TW / 2, b.y + TH / 2, xc, midY),
    };
  };
  const A = pair(at(0, -1), at(2, 1));
  const B = pair(at(cols - 2, -2), at(cols - 3, 2));
  const you = at(Math.min(2, cols - 2) + 1, -1); // where "your lot" is sealed
  const empties = new Set([`${cols - 1},1`, `1,2`]);
  return { cols, gw, x0, midY, tiles, A, B, you, empties, w, h };
}

export interface StepsHandle {
  draw(now: number, still: boolean): void;
  /** forget measured sizes (after a resize or font load) */
  reset(): void;
}

export function createSteps(
  canvases: HTMLCanvasElement[],
  items: HTMLElement[],
  labels: StepLabels,
  faces: Faces,
): StepsHandle {
  const MONO = faces.mono;
  const SANS = faces.sans;
  let sizes = new WeakMap<HTMLCanvasElement, { w: number; h: number }>();
  const grids = new WeakMap<HTMLCanvasElement, Grid>();
  const t0 = performance.now();
  let lastOn = -1;

  // mono only for real data (prices, code); plain captions use the page face
  function label(c: Ctx, text: string, x: number, y: number, align: CanvasTextAlign, a: number, color: string, face: "mono" | "sans") {
    if (a <= 0) return;
    c.globalAlpha = a;
    c.font = face === "mono" ? `400 10px ${MONO}` : `500 11px ${SANS}`;
    c.textAlign = align;
    c.textBaseline = "middle";
    c.fillStyle = color;
    c.fillText(text, x, y);
    c.globalAlpha = 1;
  }

  function channel(c: Ctx, G: Grid, a = 0.14, tint = 0) {
    if (tint > 0) {
      c.fillStyle = `rgba(212,242,74,${0.05 * tint})`;
      c.fillRect(G.x0 - 6, G.midY - 9, G.gw + 12, 18);
    }
    c.fillStyle = `rgba(255,255,255,${a})`;
    c.fillRect(G.x0 - 6, G.midY - 0.5, G.gw + 12, 1);
  }

  function base(c: Ctx, G: Grid, skip: GTile[] | null, seed: number) {
    for (const t of G.tiles) {
      if (skip && skip.some((q) => same(q, t))) continue;
      if (G.empties.has(key(t))) {
        emptyTile(c, t.x, t.y);
        continue;
      }
      sealed(c, t.x, t.y, hexFrom(seed * 97 + t.i * 13 + t.k * 7), MONO);
    }
  }

  // --- seal: your lot appears as plaintext at its limit, a scan line seals it into a digest
  function drawSeal(c: Ctx, G: Grid, t: number, n: number) {
    channel(c, G, 0.08);
    const Y = G.you;
    base(c, G, [Y], 1);
    const dig = hexFrom(500 + n);
    const plain = (a: number) => {
      c.globalAlpha = a;
      rrect(c, Y.x, Y.y, TW, TH, RAD);
      c.fillStyle = "rgba(212,242,74,.12)";
      c.fill();
      rrect(c, Y.x + 0.5, Y.y + 0.5, TW - 1, TH - 1, RAD - 0.5);
      c.strokeStyle = LIME;
      c.lineWidth = 1;
      c.stroke();
      c.font = `500 8.5px ${MONO}`;
      c.textAlign = "center";
      c.textBaseline = "middle";
      c.fillStyle = LIME;
      c.fillText(labels.sellLimit, Y.x + TW / 2, Y.y + TH / 2 + 0.5);
      c.globalAlpha = 1;
    };
    const caption = "your lot, signed at its limit";
    if (t < 200) {
      emptyTile(c, Y.x, Y.y);
      return;
    }
    if (t < 1000) {
      emptyTile(c, Y.x, Y.y, 1 - eOut(seg(t, 200, 600)));
      plain(eOut(seg(t, 200, 600)));
      label(c, caption, Y.x + TW / 2, G.midY - 9, "center", eOut(seg(t, 300, 700)) * (1 - seg(t, 1700, 2100)), "rgba(255,255,255,.8)", "sans");
      return;
    }
    if (t < 1900) {
      const k = eIO(seg(t, 1000, 1900));
      const sx = Y.x + TW * k;
      plain(1);
      c.save();
      c.beginPath();
      c.rect(Y.x - 1, Y.y - 1, sx - Y.x + 1, TH + 2);
      c.clip();
      sealed(c, Y.x, Y.y, dig, MONO);
      c.restore();
      c.fillStyle = LIME;
      c.fillRect(sx - 0.5, Y.y - 4, 1, TH + 8);
      label(c, caption, Y.x + TW / 2, G.midY - 9, "center", 1 - seg(t, 1700, 2100), "rgba(255,255,255,.8)", "sans");
      return;
    }
    if (t < 7500) {
      sealed(c, Y.x, Y.y, dig, MONO);
      const r = seg(t, 1900, 2700);
      if (r > 0 && r < 1) {
        const g = eOut(r);
        rrect(c, Y.x - 2 - 6 * g, Y.y - 2 - 6 * g, TW + 4 + 12 * g, TH + 4 + 12 * g, RAD + 3);
        c.strokeStyle = `rgba(212,242,74,${0.8 * (1 - r)})`;
        c.lineWidth = 1;
        c.stroke();
      }
      label(c, "sealed, same size as every lot", Y.x + TW / 2, G.midY - 9, "center", seg(t, 1900, 2300) * (1 - seg(t, 4200, 4800)), "rgba(255,255,255,.8)", "sans");
      return;
    }
    // it settles in step 4 and leaves the book; the slot waits for the next lot
    const k = eOut(seg(t, 7600, 8400));
    sealed(c, Y.x, Y.y, dig, MONO, 1 - k);
    emptyTile(c, Y.x, Y.y, k);
  }

  // --- batch: anyone cranks run_batch(); a ripple crosses the book; one batch bracket closes
  function drawBatch(c: Ctx, G: Grid, t: number) {
    const lt = t - WIN;
    channel(c, G, 0.08);
    base(c, G, null, 2);
    if (lt < 0 || lt > 3400) return;
    // the crank dot runs in along the channel
    const ck = eIO(seg(lt, 0, 700));
    if (lt < 1300) {
      const x = G.x0 - 6 + (G.w / 2 - G.x0 + 6) * ck;
      c.beginPath();
      c.arc(x, G.midY, 2.6, 0, Math.PI * 2);
      c.fillStyle = `rgba(212,242,74,${1 - seg(lt, 900, 1300)})`;
      c.fill();
    }
    label(c, "run_batch()", G.x0, G.midY - 9, "left", seg(lt, 0, 300) * (1 - seg(lt, 2000, 2500)), "rgba(255,255,255,.8)", "mono");
    // ripple
    const rk = seg(lt, 600, 2200);
    if (rk > 0 && rk < 1) {
      const R = eOut(rk) * Math.hypot(G.w / 2, G.h / 2) * 1.1;
      for (const tl of G.tiles) {
        if (G.empties.has(key(tl))) continue;
        const d = Math.hypot(tl.x + TW / 2 - G.w / 2, tl.y + TH / 2 - G.midY);
        const wgt = Math.exp(-Math.pow((d - R) / 40, 2));
        if (wgt < 0.03) continue;
        rrect(c, tl.x, tl.y, TW, TH, RAD);
        c.fillStyle = `rgba(255,255,255,${0.2 * (1 - rk) * wgt})`;
        c.fill();
      }
    }
    // the batch bracket
    const bk = eIO(seg(lt, 900, 1900));
    const ba = 1 - seg(lt, 2500, 3400);
    if (bk > 0 && ba > 0) {
      const x = G.x0 - 8;
      const y = G.midY - 2 * PY - TH / 2 - 8;
      const w = G.gw + 16;
      const h = 4 * PY + TH + 16;
      const per = 2 * (w + h);
      c.save();
      c.setLineDash([per * bk, per]);
      rrect(c, x + 0.5, y + 0.5, w - 1, h - 1, 10);
      c.strokeStyle = `rgba(212,242,74,${0.85 * ba})`;
      c.lineWidth = 1;
      c.stroke();
      c.restore();
    }
  }

  // --- cross: compatible pairs light, wire to the channel and meet on one price line
  function crossedPair(c: Ctx, G: Grid, P: GPair, lt: number) {
    const lightK = eOut(seg(lt, 0, 400));
    const wireK = eIO(seg(lt, 400, 1300));
    const crossK = eOut(seg(lt, 1300, 1700));
    const fade = eOut(seg(lt, 2500, 3300));
    if (wireK > 0 && fade < 1) {
      c.strokeStyle = `rgba(212,242,74,${0.95 * (1 - fade)})`;
      c.lineWidth = 1;
      c.lineCap = "butt";
      c.lineJoin = "miter";
      strokePartial(c, P.ls, 0, P.ls.len * wireK);
      strokePartial(c, P.lb, 0, P.lb.len * wireK);
    }
    if (crossK > 0 && fade < 1) {
      const rk = seg(lt, 1300, 2200);
      c.beginPath();
      c.arc(P.xc, G.midY, 4 + 10 * eOut(rk), 0, Math.PI * 2);
      c.strokeStyle = `rgba(212,242,74,${0.55 * (1 - rk) * (1 - fade)})`;
      c.lineWidth = 1;
      c.stroke();
      c.beginPath();
      c.arc(P.xc, G.midY, 3.2, 0, Math.PI * 2);
      c.fillStyle = `rgba(212,242,74,${1 - fade})`;
      c.fill();
    }
    for (const tl of [P.s, P.b]) {
      const lit = tl === P.s ? LIME : WHITE;
      const fill = crossK > 0 ? (tl === P.s ? LIME : mix(WHITE, LIME, crossK)) : mix(TILE, lit, lightK);
      sealed(c, tl.x, tl.y, hexFrom(tl.i * 31 + tl.k), MONO);
      solid(c, tl.x, tl.y, fill, 1 - fade);
      check(c, tl.x, tl.y, crossK * (1 - fade));
    }
  }

  function drawCross(c: Ctx, G: Grid, t: number) {
    const lt = t - 2 * WIN;
    const on = lt >= 0 && lt < 3300;
    const tint = on ? seg(lt, 1300, 1700) * (1 - seg(lt, 2500, 3300)) : 0;
    channel(c, G, 0.16, tint);
    base(c, G, on ? [G.A.s, G.A.b, G.B.s, G.B.b] : null, 3);
    if (!on) return;
    crossedPair(c, G, G.A, lt);
    crossedPair(c, G, G.B, lt - 220);
    label(c, labels.mid, G.x0 + G.gw + 4, G.midY - 9, "right", tint, "rgba(255,255,255,.85)", "mono");
  }

  // --- settle: both legs in one call, both tiles empty together, the improvement runs back
  function drawSettle(c: Ctx, G: Grid, t: number) {
    const lt = (t - 3 * WIN + CYCLE) % CYCLE;
    const P = G.A;
    const on = lt < 3300;
    channel(c, G, 0.16, on ? 1 - seg(lt, 1900, 2600) : 0);
    base(c, G, on ? [P.s, P.b] : null, 4);
    if (!on) return;
    const inK = eOut(seg(lt, 0, 300));
    const drain = eIO(seg(lt, 900, 1500));
    const wireA = 1 - seg(lt, 1500, 2100);
    // wires and the node from the cross
    if (wireA > 0) {
      c.strokeStyle = `rgba(212,242,74,${0.95 * inK * wireA})`;
      c.lineWidth = 1;
      strokePartial(c, P.ls, 0, P.ls.len);
      strokePartial(c, P.lb, 0, P.lb.len);
      c.beginPath();
      c.arc(P.xc, G.midY, 3.2, 0, Math.PI * 2);
      c.fillStyle = `rgba(212,242,74,${inK * wireA})`;
      c.fill();
    }
    // one execute_intents carries both legs
    const ba = 1 - seg(lt, 2000, 2700);
    label(c, "execute_intents", G.x0 + G.gw + 4, G.midY - 9, "right", seg(lt, 300, 700) * ba, "rgba(255,255,255,.85)", "mono");
    if (drain > 0 && drain < 1) {
      for (const tl of [P.s, P.b]) {
        rrect(c, tl.x - 3, tl.y - 3, TW + 6, TH + 6, RAD + 2);
        c.strokeStyle = `rgba(212,242,74,${0.7 * (1 - drain)})`;
        c.lineWidth = 1;
        c.stroke();
      }
    }
    // both legs at once: lime drains to an empty slot on the same frame
    const rk = seg(lt, 1100, 1900);
    const refill = eOut(seg(lt, 2500, 3300));
    for (const tl of [P.s, P.b]) {
      if (drain < 1) {
        solid(c, tl.x, tl.y, mix(LIME, DEEP, drain), inK);
        check(c, tl.x, tl.y, inK * (1 - drain));
      }
      emptyTile(c, tl.x, tl.y, drain * (1 - refill));
      if (rk >= 1 && refill < 1) {
        const g = 1 - seg(lt, 1900, 2500);
        if (g > 0) {
          c.beginPath();
          c.arc(tl.x + TW / 2, tl.y + TH / 2, 2.4, 0, Math.PI * 2);
          c.fillStyle = `rgba(212,242,74,${g})`;
          c.fill();
        }
      }
      if (refill > 0) sealed(c, tl.x, tl.y, hexFrom(900 + tl.i + tl.k), MONO, refill);
    }
    // the improvement runs back from the node into both slots
    if (rk > 0 && rk < 1) {
      const q = eIO(rk);
      for (const L of [P.ls, P.lb]) {
        const p = pointAt(L, L.len * (1 - q));
        c.beginPath();
        c.arc(p[0], p[1], 2.4, 0, Math.PI * 2);
        c.fillStyle = LIME;
        c.fill();
      }
    }
  }

  const drawers = [drawSeal, drawBatch, drawCross, drawSettle];

  function draw(now: number, still: boolean) {
    const t = still ? 0 : (now - t0) % CYCLE;
    const n = still ? 0 : Math.floor((now - t0) / CYCLE);
    canvases.forEach((cv, i) => {
      const { c, w, h } = fit(cv, sizes);
      c.clearRect(0, 0, w, h);
      let G = grids.get(cv);
      if (!G || G.w !== w || G.h !== h) {
        G = grid(w, h);
        grids.set(cv, G);
      }
      drawers[i](c, G, still ? STILL[i] : t, n);
    });
    if (still) return;
    const on = Math.floor(t / WIN);
    items.forEach((el, i) => el.style.setProperty("--p", String(i === on ? (t - i * WIN) / WIN : 0)));
    if (on !== lastOn) {
      items.forEach((el, i) => {
        if (i === on) el.dataset.on = "1";
        else delete el.dataset.on;
      });
      lastOn = on;
    }
  }

  return {
    draw,
    reset() {
      sizes = new WeakMap();
    },
  };
}
