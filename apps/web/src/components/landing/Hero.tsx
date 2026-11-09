"use client";

import { createContext, type ReactNode, use, useCallback, useEffect, useRef } from "react";
import { prefersReducedMotion, readFaces } from "@/components/tiles/palette";
import { createWall, type WallHandle } from "@/components/tiles/wall";

const BatchContext = createContext<() => void>(() => {});

/** Reveal the hero two frames after the wall has drawn (the inline head script also reveals it after 1.2 s). */
function revealSoon() {
  let id = requestAnimationFrame(() => {
    id = requestAnimationFrame(() => document.documentElement.classList.add("is-in"));
  });
  return () => cancelAnimationFrame(id);
}

/**
 * The first screen: the sealed wall on a canvas, the frosted panel (passed in
 * as children), and the marquee of real settlement artefacts.
 */
export function Hero({ children, marquee, marqueeLabel }: { children: ReactNode; marquee: string[]; marqueeLabel: string }) {
  const heroRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const marqueeRef = useRef<HTMLDivElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const wall = useRef<WallHandle | null>(null);

  useEffect(() => {
    const hero = heroRef.current;
    const canvas = canvasRef.current;
    const panel = panelRef.current;
    const strip = marqueeRef.current;
    const tip = tipRef.current;
    if (!hero || !canvas || !panel || !strip || !tip) return;
    wall.current = createWall({ hero, canvas, panel, marquee: strip, tip }, readFaces(), prefersReducedMotion());
    const cancelReveal = revealSoon();
    return () => {
      cancelReveal();
      wall.current?.destroy();
      wall.current = null;
    };
  }, []);

  const batch = useCallback(() => wall.current?.batch(), []);

  const items = marquee.map((text) => (
    <span key={text}>
      <i />
      {text}
    </span>
  ));

  return (
    <BatchContext value={batch}>
      <div className="hero" ref={heroRef}>
        <canvas className="wall" ref={canvasRef} aria-hidden="true" />
        <div className="scrim" aria-hidden="true" />
        <div className="stage">
          <section className="panel bb-reveal" ref={panelRef} aria-labelledby="hero-title">
            {children}
          </section>
        </div>
        <div className="tip" ref={tipRef} aria-hidden="true" />
        {/* hover or focus pauses the strip */}
        <div className="marquee" ref={marqueeRef} role="region" aria-label={marqueeLabel} tabIndex={0}>
          <div className="track">
            <div className="run">{items}</div>
            <div className="run" aria-hidden="true">
              {items}
            </div>
          </div>
        </div>
      </div>
    </BatchContext>
  );
}

/** Starts a batch on the hero wall right away. */
export function WatchBatchButton({ children }: { children: ReactNode }) {
  const batch = use(BatchContext);
  return (
    <button type="button" className="bb-link" onClick={batch}>
      {children}
    </button>
  );
}
