import { GROUND, PX, RAD, TH, TILE, TW } from "./palette";

export type Ctx = CanvasRenderingContext2D;
export type Pt = [number, number];

// ---------------------------------------------------------------- easing (no springs)

export const clamp01 = (t: number) => (t < 0 ? 0 : t > 1 ? 1 : t);
/** Progress of `t` through the window [a, b], clamped. */
export const seg = (t: number, a: number, b: number) => clamp01((t - a) / (b - a));
export const eOut = (t: number) => 1 - Math.pow(1 - clamp01(t), 3);
export const eIO = (t: number) => {
  const x = clamp01(t);
  return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
};

// ---------------------------------------------------------------- colour and noise

const hexRGB = (h: string): [number, number, number] => [
  parseInt(h.slice(1, 3), 16),
  parseInt(h.slice(3, 5), 16),
  parseInt(h.slice(5, 7), 16),
];
export function mix(a: string, b: string, t: number) {
  const A = hexRGB(a);
  const B = hexRGB(b);
  const c = (i: number) => Math.round(A[i] + (B[i] - A[i]) * t);
  return `rgb(${c(0)},${c(1)},${c(2)})`;
}

/** Deterministic noise in [0, 1), so digests and jitter hold still between frames. */
export const hash = (n: number) => {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
};
export const hexFrom = (n: number) => ((hash(n) * 0xffffff) | 0).toString(16).padStart(6, "0");
export const hexRand = () => ((Math.random() * 0xffffff) | 0).toString(16).padStart(6, "0");
export const pick = <T>(a: readonly T[]): T => a[(Math.random() * a.length) | 0];

// ---------------------------------------------------------------- shapes

export function rrect(c: Ctx, x: number, y: number, w: number, h: number, r: number) {
  c.beginPath();
  if (c.roundRect) {
    c.roundRect(x, y, w, h, r);
    return;
  }
  c.moveTo(x + r, y);
  c.arcTo(x + w, y, x + w, y + h, r);
  c.arcTo(x + w, y + h, x, y + h, r);
  c.arcTo(x, y + h, x, y, r);
  c.arcTo(x, y, x + w, y, r);
  c.closePath();
}

/** A sealed lot: tile fill, faint edge, a ciphertext digest. */
export function sealed(c: Ctx, x: number, y: number, dig: string | null, mono: string, a = 1) {
  c.globalAlpha = a;
  rrect(c, x, y, TW, TH, RAD);
  c.fillStyle = TILE;
  c.fill();
  rrect(c, x + 0.5, y + 0.5, TW - 1, TH - 1, RAD - 0.5);
  c.strokeStyle = "rgba(255,255,255,.10)";
  c.lineWidth = 1;
  c.stroke();
  if (dig) {
    c.fillStyle = "rgba(255,255,255,.25)";
    c.font = `400 8.5px ${mono}`;
    c.textAlign = "center";
    c.textBaseline = "middle";
    c.fillText(dig, x + TW / 2, y + TH / 2 + 0.5);
  }
  c.globalAlpha = 1;
}

/** A free slot: outline only. */
export function emptyTile(c: Ctx, x: number, y: number, a = 1) {
  c.globalAlpha = a;
  rrect(c, x + 0.5, y + 0.5, TW - 1, TH - 1, RAD - 0.5);
  c.strokeStyle = "rgba(255,255,255,.18)";
  c.lineWidth = 1;
  c.stroke();
  c.globalAlpha = 1;
}

export function solid(c: Ctx, x: number, y: number, fill: string, a = 1) {
  c.globalAlpha = a;
  rrect(c, x, y, TW, TH, RAD);
  c.fillStyle = fill;
  c.fill();
  c.globalAlpha = 1;
}

/** The fill check mark, drawn in the ground colour on a lit tile. */
export function check(c: Ctx, x: number, y: number, a = 1) {
  if (a <= 0) return;
  const cx = x + TW / 2;
  const cy = y + TH / 2;
  c.globalAlpha = a;
  c.beginPath();
  c.moveTo(cx - 5, cy + 0.5);
  c.lineTo(cx - 1.5, cy + 4);
  c.lineTo(cx + 5.5, cy - 3.5);
  c.strokeStyle = GROUND;
  c.lineWidth = 1.6;
  c.lineCap = "round";
  c.lineJoin = "round";
  c.stroke();
  c.globalAlpha = 1;
}

// ---------------------------------------------------------------- wires

export interface Leg {
  pts: Pt[];
  len: number;
  cum: number[];
  /** x of the gap the wire runs down */
  gx: number;
}

/** Orthogonal wire from a tile's side, down the gap beside it, along the mid channel to `xc`. */
export function leg(cx: number, cy: number, xc: number, midY: number): Leg {
  const dir = xc >= cx ? 1 : -1;
  const gx = cx + (dir * PX) / 2;
  const pts: Pt[] = [
    [cx + (dir * TW) / 2, cy],
    [gx, cy],
    [gx, midY],
    [xc, midY],
  ];
  let len = 0;
  const cum = [0];
  for (let i = 1; i < pts.length; i++) {
    len += Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]);
    cum.push(len);
  }
  return { pts, len, cum, gx };
}

export function pointAt(L: Leg, dist: number): Pt {
  const d = Math.max(0, Math.min(L.len, dist));
  for (let i = 1; i < L.pts.length; i++) {
    if (d <= L.cum[i]) {
      const a = L.pts[i - 1];
      const b = L.pts[i];
      const s = (d - L.cum[i - 1]) / (L.cum[i] - L.cum[i - 1] || 1);
      return [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s];
    }
  }
  return L.pts[L.pts.length - 1];
}

export function strokePartial(c: Ctx, L: Leg, from: number, to: number) {
  if (to <= from) return;
  c.beginPath();
  const p0 = pointAt(L, from);
  c.moveTo(p0[0], p0[1]);
  for (let i = 1; i < L.pts.length; i++) if (L.cum[i] > from && L.cum[i] < to) c.lineTo(L.pts[i][0], L.pts[i][1]);
  const p1 = pointAt(L, to);
  c.lineTo(p1[0], p1[1]);
  c.stroke();
}

// ---------------------------------------------------------------- sizing

export interface Fitted {
  c: Ctx;
  w: number;
  h: number;
}

/**
 * Size a canvas's backing store to its CSS box at the device pixel ratio
 * (capped at 2). The CSS size is measured once per layout and cached in
 * `sizes`, never per frame.
 */
export function fit(cv: HTMLCanvasElement, sizes: WeakMap<HTMLCanvasElement, { w: number; h: number }>): Fitted {
  let size = sizes.get(cv);
  if (!size) {
    size = { w: Math.max(1, cv.clientWidth), h: Math.max(1, cv.clientHeight) };
    sizes.set(cv, size);
  }
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const pw = Math.round(size.w * dpr);
  const ph = Math.round(size.h * dpr);
  if (cv.width !== pw || cv.height !== ph) {
    cv.width = pw;
    cv.height = ph;
  }
  const c = cv.getContext("2d")!;
  c.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { c, w: size.w, h: size.h };
}
