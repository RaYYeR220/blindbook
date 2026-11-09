"use client";

import { useEffect, useRef, useState } from "react";

/** Copies `text` and confirms in place for 1.6 s. */
export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [state, setState] = useState<"idle" | "done" | "failed">("idle");
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("done");
    } catch {
      setState("failed");
    }
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setState("idle"), 1600);
  };

  return (
    <button type="button" className="copy" data-done={state === "idle" ? undefined : ""} onClick={copy}>
      <span aria-live="polite">{state === "done" ? "Copied" : state === "failed" ? "Select and copy" : label}</span>
    </button>
  );
}
