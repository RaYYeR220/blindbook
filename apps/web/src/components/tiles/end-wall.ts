import { emptyTile, eOut, fit, hash, hexFrom, hexRand, pick, rrect, sealed, seg } from "./draw";
import { PX, PY, RAD, TH, TW } from "./palette";

/*
  The closing strip of the wall behind the last call to action: sealed lots
  keep arriving, a few leave, and one ripple crosses it per 7 s batch.
*/

interface ETile {
  x: number;
  y: number;
  dig: string;
  next: string;
  state: "sealed" | "empty";
  busy: boolean;
}

interface Move {
  t: ETile;
  s: number;
  dir: "in" | "out";
}

export interface EndWallHandle {
  layout(): void;
  draw(now: number): void;
  reset(): void;
}

const ALPHA = 0.85;

export function createEndWall(cv: HTMLCanvasElement, mono: string, reduced: boolean): EndWallHandle {
  let sizes = new WeakMap<HTMLCanvasElement, { w: number; h: number }>();
  let tiles: ETile[] = [];
  let moves: Move[] = [];
  let last = 0;
  const t0 = performance.now();

  function layout() {
    const w = cv.clientWidth;
    const h = cv.clientHeight;
    tiles = [];
    const cols = Math.ceil(w / PX / 2) + 1;
    const rows = Math.ceil(h / PY / 2) + 1;
    for (let k = -rows; k <= rows; k++) {
      for (let j = -cols; j <= cols; j++) {
        tiles.push({
          x: Math.round(w / 2 + j * PX - TW / 2),
          y: Math.round(h / 2 + k * PY - TH / 2),
          dig: hexFrom(j * 41 + k * 7 + 3),
          next: "",
          state: hash(j * 3 + k * 17) < 0.03 ? "empty" : "sealed",
          busy: false,
        });
      }
    }
    moves = [];
  }

  function draw(now: number) {
    const { c, w, h } = fit(cv, sizes);
    c.clearRect(0, 0, w, h);
    for (const t of tiles) {
      if (t.busy) continue;
      if (t.state === "sealed") sealed(c, t.x, t.y, t.dig, mono, ALPHA);
      else emptyTile(c, t.x, t.y, ALPHA);
    }
    if (reduced) return;
    // sealed lots keep arriving; a few leave (settled elsewhere)
    if (now - last > 520) {
      last = now;
      const vis = tiles.filter((t) => !t.busy && t.x > -TW && t.x < w && t.y > -TH && t.y < h);
      const empties = vis.filter((t) => t.state === "empty");
      if (empties.length) {
        const t = pick(empties);
        t.busy = true;
        t.next = hexRand();
        moves.push({ t, s: now, dir: "in" });
      }
      if (empties.length < vis.length * 0.025) {
        const pool = vis.filter((t) => t.state === "sealed");
        if (pool.length) {
          const t = pick(pool);
          t.busy = true;
          moves.push({ t, s: now, dir: "out" });
        }
      }
    }
    moves = moves.filter((m) => {
      const k = eOut(seg(now - m.s, 0, 700));
      if (m.dir === "in") {
        emptyTile(c, m.t.x, m.t.y, ALPHA * (1 - k));
        sealed(c, m.t.x, m.t.y, m.t.next, mono, ALPHA * k);
      } else {
        sealed(c, m.t.x, m.t.y, m.t.dig, mono, ALPHA * (1 - k));
        emptyTile(c, m.t.x, m.t.y, ALPHA * k);
      }
      if (k < 1) return true;
      m.t.busy = false;
      if (m.dir === "in") {
        m.t.state = "sealed";
        m.t.dig = m.t.next;
      } else m.t.state = "empty";
      return false;
    });
    // one ripple per 7 s batch, from the centre
    const k = seg((now - t0) % 7000, 0, 2200);
    if (k > 0 && k < 1) {
      const R = eOut(k) * Math.hypot(w / 2, h / 2);
      for (const t of tiles) {
        if (t.busy) continue;
        const d = Math.hypot(t.x + TW / 2 - w / 2, t.y + TH / 2 - h / 2);
        const wgt = Math.exp(-Math.pow((d - R) / 70, 2));
        if (wgt < 0.03) continue;
        rrect(c, t.x, t.y, TW, TH, RAD);
        c.fillStyle = `rgba(255,255,255,${0.14 * (1 - k) * wgt})`;
        c.fill();
      }
    }
  }

  return {
    layout,
    draw,
    reset() {
      sizes = new WeakMap();
    },
  };
}
