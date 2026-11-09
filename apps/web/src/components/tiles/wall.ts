import {
  check,
  type Ctx,
  eIO,
  emptyTile,
  eOut,
  hexRand,
  type Leg,
  leg,
  mix,
  pick,
  pointAt,
  rrect,
  sealed,
  seg,
  strokePartial,
} from "./draw";
import { type Faces, GROUND, LIME, PX, PY, RAD, TH, TILE, TW, WHITE } from "./palette";

/*
  The hero: a wall of identical sealed lots. Every tile is the same size on
  purpose (fixed-length ciphertext with bucket padding). Every 7 s a batch
  ticks: a ripple leaves the centre, 2 to 4 compatible pairs light up (sell
  lime, buy white), wire through the gaps to the mid channel and meet there,
  settle as one (both legs at once), and a rebate dot runs back into each
  slot. Fresh sealed lots fill the empty slots. Sizes, limits and the mid are
  never drawn; the batch is illustrative.
*/

type State = "sealed" | "empty";

interface Tile {
  cx: number;
  cy: number;
  x: number;
  y: number;
  k: number;
  j: number;
  side: "sell" | "buy";
  dig: string;
  next: string | null;
  state: State;
  busy: boolean;
}

interface Pair {
  s: Tile;
  b: Tile;
  xc: number;
  ls: Leg;
  lb: Leg;
  delay: number;
  lx: number | null;
  done: boolean;
}

interface Refill {
  t: Tile;
  start: number;
  dir: "in" | "out";
}

interface Box {
  l: number;
  t: number;
  r: number;
  b: number;
}

export interface WallElements {
  hero: HTMLElement;
  canvas: HTMLCanvasElement;
  panel: HTMLElement;
  marquee: HTMLElement;
  tip: HTMLElement;
}

export interface WallHandle {
  /** Start a batch now (or redraw the still frame under reduced motion). */
  batch(): void;
  destroy(): void;
}

const CYCLE = 7000;
const PRINT = "crossed at mid · both legs · 1 tx";
/** Pair timeline, ms after the pair's own delay. */
const T = {
  light: [500, 900],
  wire: [900, 2100],
  cross: [2100, 2450],
  settle: [3500, 4100],
  rebate: [3700, 4500],
  refill: [5200, 5800],
} as const;

export function createWall(el: WallElements, faces: Faces, reduced: boolean): WallHandle {
  const { hero, canvas: cv, panel, marquee, tip } = el;
  const ctx = cv.getContext("2d")!;
  const base = document.createElement("canvas");
  const bctx = base.getContext("2d")!;
  const MONO = faces.mono;

  let W = 0;
  let H = 0;
  let DPR = 1;
  let bandY = 0;
  let topLimit = 0;
  let botLimit = 0;
  let P: Box = { l: 0, t: 0, r: 0, b: 0 };
  let tiles: Tile[] = [];
  let byKey = new Map<string, Tile>();
  let pairs: Pair[] = [];
  let refills: Refill[] = [];
  let batchStart = -1;
  let ripple = { x: 0, y: 0 };
  let hover: Tile | null = null;
  let clickUntil = 0;
  let lastIdleFill = 0;
  let forceBatch = false;
  let started = 0;
  let labelSpans: [number, number][] = [];
  let raf = 0;
  let destroyed = false;

  const clearOfLabels = (a: number, z: number, m: number) => labelSpans.every(([l, r]) => z < l - m || a > r + m);
  const outside = (x0: number, y0: number, x1: number, y1: number, m: number) =>
    x1 < P.l - m || x0 > P.r + m || y1 < P.t - m || y0 > P.b + m;

  // ---------------------------------------------------------------- painters

  const clearTile = (c: Ctx, t: Tile) => {
    c.fillStyle = GROUND;
    c.fillRect(t.x - 1.5, t.y - 1.5, TW + 3, TH + 3);
  };
  const paintBase = (t: Tile) => {
    clearTile(bctx, t);
    if (t.state === "sealed") sealed(bctx, t.x, t.y, t.dig, MONO);
    else emptyTile(bctx, t.x, t.y);
  };

  // ---------------------------------------------------------------- layout

  const rel = (node: HTMLElement): Box => {
    const h = hero.getBoundingClientRect();
    const r = node.getBoundingClientRect();
    return { l: r.left - h.left, t: r.top - h.top, r: r.right - h.left, b: r.bottom - h.top };
  };

  function layout() {
    DPR = Math.min(2, window.devicePixelRatio || 1);
    const hr = hero.getBoundingClientRect();
    W = hr.width;
    H = hr.height;
    cv.width = Math.round(W * DPR);
    cv.height = Math.round(H * DPR);
    base.width = cv.width;
    base.height = cv.height;
    P = rel(panel);
    topLimit = 70;
    botLimit = rel(marquee).t;

    // the mid channel runs through the panel on wide screens, through the
    // larger free band above or below it when the panel spans the width
    const fullWidth = P.l < 90 && P.r > W - 90;
    if (!fullWidth) bandY = Math.round(H * 0.5);
    else {
      const below = botLimit - P.b;
      const above = P.t - topLimit;
      bandY = Math.round(below >= above ? P.b + below / 2 : topLimit + above / 2);
    }
    ripple = { x: (P.l + P.r) / 2, y: (P.t + P.b) / 2 };

    tiles = [];
    byKey = new Map();
    const cols = Math.ceil(W / PX / 2) + 1;
    const up = Math.ceil(bandY / PY) + 1;
    const dn = Math.ceil((H - bandY) / PY) + 1;
    for (let k = -up; k <= dn; k++) {
      if (k === 0) continue; // the mid channel: one row left open
      const cy = bandY + k * PY;
      for (let j = -cols; j <= cols; j++) {
        const cx = Math.round(W / 2 + j * PX);
        const t: Tile = {
          cx,
          cy,
          x: cx - TW / 2,
          y: cy - TH / 2,
          k,
          j,
          side: k < 0 ? "sell" : "buy",
          dig: hexRand(),
          next: null,
          state: Math.random() < 0.025 ? "empty" : "sealed",
          busy: false,
        };
        tiles.push(t);
        byKey.set(`${j},${k}`, t);
      }
    }
    pairs = [];
    refills = [];
    buildBase();
  }

  function buildBase() {
    bctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    bctx.fillStyle = GROUND;
    bctx.fillRect(0, 0, W, H);
    for (const t of tiles) {
      if (t.y > H + 4 || t.y + TH < -4) continue;
      if (t.state === "sealed") sealed(bctx, t.x, t.y, t.dig, MONO);
      else emptyTile(bctx, t.x, t.y);
    }
    // the mid channel
    bctx.fillStyle = "rgba(212,242,74,.035)";
    bctx.fillRect(0, bandY - 11, W, 22);
    bctx.fillStyle = "rgba(255,255,255,.16)";
    bctx.fillRect(0, bandY - 0.5, W, 1);
    // static labels ride the upper half of the channel, in free stretches only;
    // wires run on the centre line
    bctx.font = `400 10px ${MONO}`;
    bctx.textBaseline = "middle";
    const labels: [string, "left" | "right"][] = [
      ["mid · 1Click-signed", "right"],
      ["illustrative batch", "left"],
    ];
    labelSpans = [];
    for (const [label, side] of labels) {
      const lw = bctx.measureText(label).width;
      const x = side === "right" ? W - 16 - lw : 16;
      if (!outside(x - 4, bandY - 20, x + lw + 4, bandY - 6, 6)) continue;
      labelSpans.push([x - 4, x + lw + 4]);
      bctx.fillStyle = "rgba(255,255,255,.5)";
      bctx.textAlign = "left";
      bctx.fillText(label, x, bandY - 12.5);
    }
  }

  // ---------------------------------------------------------------- pairs

  const tileOK = (t: Tile) =>
    t.state === "sealed" &&
    !t.busy &&
    t.y > topLimit + 8 &&
    t.y + TH < botLimit - 8 &&
    t.x > 8 &&
    t.x + TW < W - 8 &&
    outside(t.x, t.y, t.x + TW, t.y + TH, 14) &&
    Math.abs(t.k) <= 4;

  function pickPairs(n: number): Pair[] {
    const out: Pair[] = [];
    const used: [number, number][] = [];
    const sells = tiles.filter((t) => t.side === "sell" && tileOK(t));
    const buys = tiles.filter((t) => t.side === "buy" && tileOK(t));
    for (let tries = 0; tries < 400 && out.length < n && sells.length && buys.length; tries++) {
      const s = pick(sells);
      if (s.busy) continue;
      const cands = buys.filter((b) => !b.busy && Math.abs(b.j - s.j) >= 2 && Math.abs(b.j - s.j) <= 4);
      if (!cands.length) continue;
      const b = pick(cands);
      const xc = (s.cx + b.cx) / 2;
      const ls = leg(s.cx, s.cy, xc, bandY);
      const lb = leg(b.cx, b.cy, xc, bandY);
      const lo = Math.min(s.cx, b.cx) - 30;
      const hi = Math.max(s.cx, b.cx) + 30;
      if (used.some(([a, z]) => !(hi < a || lo > z))) continue;
      if (!outside(ls.gx - 1, Math.min(s.cy, bandY), ls.gx + 1, Math.max(s.cy, bandY), 10)) continue;
      if (!outside(lb.gx - 1, Math.min(b.cy, bandY), lb.gx + 1, Math.max(b.cy, bandY), 10)) continue;
      if (!outside(lo, bandY - 2, hi, bandY + 2, 10)) continue;
      if (!clearOfLabels(ls.gx, ls.gx, 6)) continue; // a sell wire must not cut through a label
      used.push([lo, hi]);
      s.busy = b.busy = true;
      out.push({ s, b, xc, ls, lb, delay: out.length * 240, lx: null, done: false });
    }
    return out;
  }

  /** One pair per batch carries the print, in the lower half of the channel, clear of every buy wire. */
  function placePrint() {
    ctx.font = `400 10px ${MONO}`;
    const w = ctx.measureText(PRINT).width;
    const fits = (x: number) =>
      x > 6 &&
      x + w < W - 6 &&
      outside(x - 4, bandY + 5, x + w + 4, bandY + 21, 6) &&
      pairs.every((q) => q.lb.gx < x - 8 || q.lb.gx > x + w + 8);
    for (const p of pairs) {
      for (const x of [p.xc + 10, p.xc - 10 - w, p.xc - w / 2]) {
        if (fits(x)) {
          p.lx = x;
          return;
        }
      }
    }
  }

  function settleTile(t: Tile, dig: string) {
    t.state = "sealed";
    t.dig = dig;
    t.busy = false;
    t.next = null;
    paintBase(t);
  }

  function finishPairs() {
    for (const p of pairs) {
      if (p.done) continue;
      settleTile(p.s, hexRand());
      settleTile(p.b, hexRand());
      p.done = true;
    }
    pairs = [];
  }

  function startBatch(now: number) {
    finishPairs();
    batchStart = now;
    pairs = pickPairs(3 + ((Math.random() * 2) | 0));
    placePrint();
  }

  // ---------------------------------------------------------------- frame parts

  function drawPair(p: Pair, lt: number) {
    const c = ctx;
    if (lt < T.light[0]) return;
    const lightK = eOut(seg(lt, ...T.light));
    const crossK = eOut(seg(lt, ...T.cross));
    const settleK = eIO(seg(lt, ...T.settle));
    const refillK = eOut(seg(lt, ...T.refill));

    // wires
    const wireK = eIO(seg(lt, ...T.wire));
    const wireA = 1 - seg(lt, 3500, 4300);
    if (wireK > 0 && wireA > 0) {
      c.strokeStyle = `rgba(212,242,74,${0.95 * wireA})`;
      c.lineWidth = 1;
      c.lineCap = "butt";
      c.lineJoin = "miter";
      strokePartial(c, p.ls, 0, p.ls.len * wireK);
      strokePartial(c, p.lb, 0, p.lb.len * wireK);
    }
    // meeting node on the mid channel
    if (lt >= T.cross[0] && wireA > 0) {
      const ringK = seg(lt, T.cross[0], T.cross[0] + 900);
      c.beginPath();
      c.arc(p.xc, bandY, 4 + 11 * eOut(ringK), 0, Math.PI * 2);
      c.strokeStyle = `rgba(212,242,74,${0.55 * (1 - ringK) * wireA})`;
      c.lineWidth = 1;
      c.stroke();
      c.beginPath();
      c.arc(p.xc, bandY, 3.2, 0, Math.PI * 2);
      c.fillStyle = `rgba(212,242,74,${wireA})`;
      c.fill();
    }
    if (p.lx != null) {
      const a = seg(lt, 2300, 2700) * (1 - seg(lt, 3900, 4400));
      if (a > 0) {
        c.font = `400 10px ${MONO}`;
        c.textBaseline = "middle";
        c.textAlign = "left";
        c.fillStyle = `rgba(212,242,74,${a})`;
        c.fillText(PRINT, p.lx, bandY + 13.5);
      }
    }

    // tiles
    for (const t of [p.s, p.b]) {
      const lit = t === p.s ? LIME : WHITE;
      clearTile(c, t);
      if (lt < T.settle[0]) {
        const crossedFill = t === p.s ? LIME : mix(WHITE, LIME, crossK);
        const fill = lt < T.cross[0] ? mix(TILE, lit, lightK) : crossedFill;
        rrect(c, t.x, t.y, TW, TH, RAD);
        c.fillStyle = fill;
        c.fill();
        if (lightK < 1) {
          c.globalAlpha = 1 - lightK;
          c.fillStyle = "rgba(255,255,255,.25)";
          c.font = `400 8.5px ${MONO}`;
          c.textAlign = "center";
          c.textBaseline = "middle";
          c.fillText(t.dig, t.cx, t.cy + 0.5);
          c.globalAlpha = 1;
        }
        if (crossK > 0) check(c, t.x, t.y, crossK);
      } else {
        // settle: both legs at once, the fill drains to an empty outline
        if (settleK < 1) {
          rrect(c, t.x, t.y, TW, TH, RAD);
          c.fillStyle = mix(LIME, GROUND, settleK);
          c.fill();
          check(c, t.x, t.y, 1 - settleK);
        }
        emptyTile(c, t.x, t.y, settleK);
        // the rebate dot arrives home
        if (seg(lt, ...T.rebate) >= 1) {
          const glow = 1 - seg(lt, T.rebate[1], T.rebate[1] + 700);
          if (glow > 0 && refillK === 0) {
            c.beginPath();
            c.arc(t.cx, t.cy, 2.4, 0, Math.PI * 2);
            c.fillStyle = `rgba(212,242,74,${glow})`;
            c.fill();
          }
        }
        if (refillK > 0) sealed(c, t.x, t.y, t.next, MONO, refillK);
      }
    }
    // rebate dots travel back from the mid node into both slots
    const rk = seg(lt, ...T.rebate);
    if (rk > 0 && rk < 1) {
      const q = eIO(rk);
      for (const L of [p.ls, p.lb]) {
        const pt = pointAt(L, L.len * (1 - q));
        c.beginPath();
        c.arc(pt[0], pt[1], 2.4, 0, Math.PI * 2);
        c.fillStyle = LIME;
        c.fill();
      }
    }
    if (refillK >= 1 && !p.done) {
      settleTile(p.s, p.s.next ?? hexRand());
      settleTile(p.b, p.b.next ?? hexRand());
      p.done = true;
    }
  }

  function drawRipple(bt: number) {
    const k = seg(bt, 0, 2000);
    if (k <= 0 || k >= 1) return;
    const maxR = Math.hypot(Math.max(ripple.x, W - ripple.x), Math.max(ripple.y, H - ripple.y));
    const r = eOut(k) * maxR;
    const amp = 0.16 * (1 - k);
    for (const t of tiles) {
      if (t.busy || t.state !== "sealed") continue;
      const d = Math.hypot(t.cx - ripple.x, t.cy - ripple.y);
      const w = Math.exp(-Math.pow((d - r) / 70, 2));
      if (w < 0.03) continue;
      rrect(ctx, t.x, t.y, TW, TH, RAD);
      ctx.fillStyle = `rgba(255,255,255,${amp * w})`;
      ctx.fill();
    }
  }

  function drawRefills(now: number) {
    // sealed lots arriving: empty slots fill with fresh ciphertext
    if (!reduced && now - lastIdleFill > 420) {
      lastIdleFill = now;
      const onScreen = (t: Tile) => t.y < H && t.y > -TH && !t.busy;
      const empties = tiles.filter((t) => t.state === "empty" && onScreen(t));
      if (empties.length) {
        const t = pick(empties);
        t.busy = true;
        t.next = hexRand();
        refills.push({ t, start: now, dir: "in" });
      }
      // a few lots leave (settled or self-cancelled elsewhere), so arrivals keep happening
      const visible = tiles.filter(onScreen);
      if (empties.length < visible.length * 0.02) {
        const sealedOnes = visible.filter((q) => q.state === "sealed");
        if (sealedOnes.length) {
          const t = pick(sealedOnes);
          t.busy = true;
          refills.push({ t, start: now, dir: "out" });
        }
      }
    }
    refills = refills.filter((r) => {
      const k = eOut(seg(now - r.start, 0, 700));
      clearTile(ctx, r.t);
      if (r.dir === "in") {
        emptyTile(ctx, r.t.x, r.t.y, 1 - k);
        sealed(ctx, r.t.x, r.t.y, r.t.next, MONO, k);
      } else {
        sealed(ctx, r.t.x, r.t.y, r.t.dig, MONO, 1 - k);
        emptyTile(ctx, r.t.x, r.t.y, k);
      }
      if (k < 1) return true;
      if (r.dir === "in") {
        r.t.state = "sealed";
        r.t.dig = r.t.next ?? r.t.dig;
      } else r.t.state = "empty";
      r.t.busy = false;
      r.t.next = null;
      paintBase(r.t);
      return false;
    });
  }

  function drawHover() {
    if (!hover) return;
    const t = hover;
    rrect(ctx, t.x - 1.5, t.y - 1.5, TW + 3, TH + 3, RAD + 1.5);
    ctx.strokeStyle = "rgba(255,255,255,.75)";
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  // ---------------------------------------------------------------- loop

  // the page scrolls past the hero: stop drawing while it is off screen
  let heroOn = true;
  let parked = false;
  const io = new IntersectionObserver(([e]) => {
    heroOn = e.isIntersecting;
    if (heroOn && parked && !reduced && !destroyed) {
      parked = false;
      raf = requestAnimationFrame(frame);
    }
  });
  io.observe(hero);

  function frame(now: number) {
    if (destroyed) return;
    if (!heroOn) {
      parked = true;
      return;
    }
    if (!started) started = now;
    if (forceBatch || (batchStart < 0 && now - started > 700) || (batchStart >= 0 && now - batchStart >= CYCLE)) {
      forceBatch = false;
      startBatch(now);
    }
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.drawImage(base, 0, 0);
    ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    const bt = batchStart < 0 ? -1 : now - batchStart;
    drawRefills(now);
    if (bt >= 0) drawRipple(bt);
    for (const p of pairs) for (const t of [p.s, p.b]) if (!t.next) t.next = hexRand();
    for (const p of pairs) drawPair(p, bt - p.delay);
    drawHover();
    raf = requestAnimationFrame(frame);
  }

  /** Reduced motion: one still frame of the cross. */
  function still() {
    if (!pairs.length) {
      pairs = pickPairs(3);
      for (const p of pairs) for (const t of [p.s, p.b]) t.next = hexRand();
      placePrint();
    }
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.drawImage(base, 0, 0);
    ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    for (const p of pairs) drawPair(p, 3000);
    drawHover();
  }

  // ---------------------------------------------------------------- probe a sealed lot

  function tileAt(x: number, y: number): Tile | null {
    if (y > botLimit) return null;
    if (x >= P.l && x <= P.r && y >= P.t && y <= P.b) return null;
    const j = Math.round((x - W / 2) / PX);
    const k = Math.round((y - bandY) / PY);
    const t = byKey.get(`${j},${k}`);
    if (!t) return null;
    return x >= t.x - 4 && x <= t.x + TW + 4 && y >= t.y - 4 && y <= t.y + TH + 4 ? t : null;
  }

  /** [emphasised part, rest] */
  function tipText(t: Tile, now: number): [string, string] {
    if (now < clickUntil) return ["only its owner can open it", ""];
    if (pairs.some((q) => (q.s === t || q.b === t) && !q.done)) return ["crossed at mid", " · both legs · 1 tx · illustrative"];
    if (t.state === "empty") return ["", "settled · slot free for the next sealed lot"];
    return ["sealed lot", " · size hidden · limit hidden"];
  }

  function showTip(e: PointerEvent, t: Tile) {
    const h = hero.getBoundingClientRect();
    const x = e.clientX - h.left;
    const y = e.clientY - h.top;
    const [strong, rest] = tipText(t, performance.now());
    const b = document.createElement("b");
    b.textContent = strong;
    tip.replaceChildren(...(strong ? [b] : []), rest);
    tip.dataset.on = "1";
    const tw = tip.offsetWidth;
    const th = tip.offsetHeight;
    let tx = x + 14;
    let ty = y + 16;
    if (tx + tw > W - 8) tx = x - 14 - tw;
    if (ty + th > botLimit - 6) ty = y - 16 - th;
    tip.style.transform = `translate(${Math.max(8, tx)}px, ${ty}px)`;
  }
  const hideTip = () => delete tip.dataset.on;

  const local = (e: PointerEvent) => {
    const h = hero.getBoundingClientRect();
    return tileAt(e.clientX - h.left, e.clientY - h.top);
  };
  const onMove = (e: PointerEvent) => {
    const t = local(e);
    hover = t;
    cv.style.cursor = t ? "pointer" : "default";
    if (t) showTip(e, t);
    else hideTip();
    if (reduced) still();
  };
  const onLeave = () => {
    hover = null;
    hideTip();
    if (reduced) still();
  };
  const onDown = (e: PointerEvent) => {
    const t = local(e);
    if (!t) return;
    hover = t;
    clickUntil = performance.now() + 1600;
    showTip(e, t);
    if (reduced) still();
  };
  cv.addEventListener("pointermove", onMove);
  cv.addEventListener("pointerleave", onLeave);
  cv.addEventListener("pointerdown", onDown);

  // ---------------------------------------------------------------- boot

  function boot() {
    if (destroyed) return;
    layout();
    batchStart = -1;
    started = 0;
    if (reduced) still();
  }
  let resizeTimer = 0;
  const onResize = () => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(boot, 120);
  };
  window.addEventListener("resize", onResize);

  boot();
  // canvas text uses the webfont: redraw once it has loaded
  document.fonts?.ready.then(boot);
  if (!reduced) raf = requestAnimationFrame(frame);

  return {
    batch() {
      if (reduced) still();
      else forceBatch = true;
    },
    destroy() {
      destroyed = true;
      cancelAnimationFrame(raf);
      window.clearTimeout(resizeTimer);
      window.removeEventListener("resize", onResize);
      io.disconnect();
      cv.removeEventListener("pointermove", onMove);
      cv.removeEventListener("pointerleave", onLeave);
      cv.removeEventListener("pointerdown", onDown);
    },
  };
}
