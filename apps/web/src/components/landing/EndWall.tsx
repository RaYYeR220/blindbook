"use client";

import { useEffect, useRef } from "react";
import { createEndWall } from "@/components/tiles/end-wall";
import { prefersReducedMotion, readFaces } from "@/components/tiles/palette";
import { onRelayout, register } from "@/components/tiles/scheduler";

/** A quiet strip of the sealed wall behind the closing call to action. */
export function EndWall() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const cv = ref.current;
    const section = cv?.parentElement;
    if (!cv || !section) return;
    const engine = createEndWall(cv, readFaces().mono, prefersReducedMotion());
    const boot = () => {
      engine.reset();
      engine.layout();
      engine.draw(performance.now());
    };
    boot();
    const offRelayout = onRelayout(boot);
    const unregister = register({ el: section, frame: (now) => engine.draw(now) });
    return () => {
      offRelayout();
      unregister();
    };
  }, []);

  return <canvas className="end-wall" ref={ref} aria-hidden="true" />;
}
