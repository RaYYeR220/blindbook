"use client";

import { useEffect, useLayoutEffect, useRef } from "react";
import { check, eIO, emptyTile, fit, hexFrom, mix, rrect, sealed, seg } from "@/components/tiles/draw";
import { LIME, readFaces, TH, TILE, TW } from "@/components/tiles/palette";
import { onRelayout, register } from "@/components/tiles/scheduler";

/**
 * The demo, drawn small: the house's lot and yours seal, drop into the
 * enclave when the batch is cranked, cross at the mid and settle. It follows
 * the progress steps the page observed on chain; it never runs ahead of them.
 */
export function DemoScene({ done, failed }: { done: number; failed: boolean }) {
  const cvRef = useRef<HTMLCanvasElement>(null);
  const state = useRef({ done, failed, at: [0, 0, 0, 0, 0, 0] as number[] });
  const sizes = useRef(new WeakMap<HTMLCanvasElement, { w: number; h: number }>());
  const drawRef = useRef<(now: number) => void>(() => {});

  // record when each step landed, so moves ease from that moment
  useLayoutEffect(() => {
    const s = state.current;
    const t = performance.now();
    for (let i = s.done + 1; i <= done; i++) s.at[i] = t;
    if (done < s.done) s.at = [0, 0, 0, 0, 0, 0];
    s.done = done;
    s.failed = failed;
    drawRef.current(t);
  }, [done, failed]);

  useEffect(() => {
    const cv = cvRef.current;
    if (!cv) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const faces = readFaces();

    const draw = (now: number) => {
      const { c, w, h } = fit(cv, sizes.current);
      const { done: d, failed: f, at } = state.current;
      const since = (i: number) => (reduced ? 1e9 : now - at[i]);
      c.clearRect(0, 0, w, h);

      const y = Math.round(h / 2 - TH / 2) - 4;
      const boxW = Math.min(150, w * 0.36);
      const boxL = Math.round(w / 2 - boxW / 2);
      const boxR = boxL + boxW;
      const mid = w / 2;

      // the enclave: a hairline box with the mid tick, lime once it has crossed
      const crossed = d >= 4 ? eIO(seg(since(4), 0, 400)) : 0;
      const settled = d >= 5 ? eIO(seg(since(5), 0, 500)) : 0;
      rrect(c, boxL + 0.5, y - 11.5, boxW - 1, TH + 22, 6);
      c.strokeStyle = settled > 0 ? mix("#5b64e4", LIME, settled) : d >= 3 ? "rgba(255,255,255,.34)" : "rgba(255,255,255,.16)";
      c.lineWidth = 1;
      c.stroke();
      c.beginPath();
      c.moveTo(mid + 0.5, y - 4);
      c.lineTo(mid + 0.5, y + TH + 4);
      c.strokeStyle = crossed > 0 ? `rgba(212,242,74,${0.35 + 0.5 * crossed})` : "rgba(255,255,255,.18)";
      c.stroke();

      c.font = `500 10px ${faces.mono}`;
      c.textBaseline = "alphabetic";
      c.textAlign = "center";
      c.fillStyle = "rgba(255,255,255,.55)";
      c.fillText(d >= 4 ? "mid" : "enclave", mid, y + TH + 25);

      // each lot: from its rest position into the box, then to the mid
      const restL = Math.max(4, boxL - TW - 40);
      const restR = Math.min(w - TW - 4, boxR + 40);
      const inL = boxL + 12;
      const inR = boxR - 12 - TW;
      const meetL = mid - TW - 3;
      const meetR = mid + 3;
      const move = d >= 3 ? eIO(seg(since(3), 0, 700)) : 0;
      const meet = d >= 4 ? eIO(seg(since(4), 0, 500)) : 0;
      const xL = restL + (inL - restL) * move + (meetL - inL) * meet;
      const xR = restR + (inR - restR) * move + (meetR - inR) * meet;

      const lot = (x: number, sealedAt: number, seed: number, label: string) => {
        const alpha = f ? 0.45 : 1;
        if (d < sealedAt) {
          const pulse = !reduced && d === sealedAt - 1 && !f ? 0.55 + 0.45 * Math.sin(now / 260) : 0.8;
          emptyTile(c, x, y, pulse * alpha);
        } else if (crossed > 0) {
          rrect(c, x, y, TW, TH, 3);
          c.fillStyle = mix(TILE, LIME, crossed);
          c.globalAlpha = alpha;
          c.fill();
          c.globalAlpha = 1;
          check(c, x, y, crossed * alpha);
        } else {
          const dig = reduced || f ? hexFrom(seed) : hexFrom(seed + Math.floor(now / 140));
          sealed(c, x, y, dig.slice(0, 4), faces.mono, alpha);
        }
        if (move < 0.5) {
          c.font = `500 10px ${faces.sans}`;
          c.fillStyle = `rgba(255,255,255,${0.55 * (1 - move * 2)})`;
          c.textAlign = "center";
          c.fillText(label, x + TW / 2, y + TH + 25);
        }
      };
      lot(xL, 1, 11, "house");
      lot(xR, 2, 47, "your lot");
    };

    drawRef.current = draw;
    draw(performance.now());
    const off = onRelayout(() => {
      sizes.current = new WeakMap();
      draw(performance.now());
    });
    const unregister = register({ el: cv, frame: draw });
    return () => {
      drawRef.current = () => {};
      off();
      unregister();
    };
  }, []);

  return <canvas ref={cvRef} className="demo-cv" aria-hidden="true" />;
}
