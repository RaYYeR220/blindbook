"use client";

import { type ReactNode, useEffect, useRef } from "react";
import { readFaces } from "@/components/tiles/palette";
import { onRelayout, register } from "@/components/tiles/scheduler";
import { createSteps, type StepLabels } from "@/components/tiles/steps";

export interface Step {
  title: string;
  body: ReactNode;
}

/** The four steps of a batch, each over a small canvas that replays its part of the cycle. */
export function StepScenes({ steps, labels }: { steps: Step[]; labels: StepLabels }) {
  const listRef = useRef<HTMLOListElement>(null);
  const { sellLimit, mid } = labels;

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const items = [...list.querySelectorAll<HTMLElement>(":scope > li")];
    const canvases = items.map((li) => li.querySelector("canvas")!);
    const engine = createSteps(canvases, items, { sellLimit, mid }, readFaces());
    // a still key frame per scene until the list is on screen
    const still = () => {
      engine.reset();
      engine.draw(performance.now(), true);
    };
    still();
    const offRelayout = onRelayout(still);
    const unregister = register({ el: list, frame: (now) => engine.draw(now, false) });
    return () => {
      offRelayout();
      unregister();
    };
  }, [sellLimit, mid]);

  return (
    <ol className="steps" ref={listRef}>
      {steps.map((step, i) => (
        <li className="step" key={step.title}>
          <div className="step-top">
            <span className="step-n">{i + 1}</span>
            <span className="step-bar">
              <i />
            </span>
          </div>
          <canvas className="scene" aria-hidden="true" />
          <h3 className="h3">{step.title}</h3>
          <p>{step.body}</p>
        </li>
      ))}
    </ol>
  );
}
