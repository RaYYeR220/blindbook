"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

export interface Polled<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  /** when the last successful read finished, ms since epoch */
  at: number | null;
  refresh: () => Promise<void>;
}

/**
 * Read `fn` now and every `ms` while the tab is visible. Keeps the last good
 * value through a failed refresh, so a flaky RPC doesn't blank the screen.
 */
export function usePoll<T>(fn: () => Promise<T>, ms: number, enabled = true, key = ""): Polled<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [at, setAt] = useState<number | null>(null);
  const fnRef = useRef(fn);
  useEffect(() => {
    fnRef.current = fn;
  });
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const v = await fnRef.current();
      if (mine !== seq.current) return;
      setData(v);
      setError(null);
      setAt(Date.now());
    } catch (e) {
      if (mine !== seq.current) return;
      setError((e as Error).message);
    } finally {
      if (mine === seq.current) setLoading(false);
    }
  }, []);

  // a new key is a different subject (another account): drop the old value
  useEffect(() => {
    seq.current++;
    setData(null);
    setError(null);
    setAt(null);
  }, [key]);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }
    setLoading(true);
    void refresh();
    const tick = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    const id = window.setInterval(tick, ms);
    document.addEventListener("visibilitychange", tick);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", tick);
    };
  }, [enabled, ms, refresh, key]);

  return { data, error, loading, at, refresh };
}

/**
 * The current time, updated every `ms` while `ticking`. Null until mounted,
 * so the server render and the first client render agree.
 */
export function useNow(ms = 1000, ticking = true): number | null {
  const [now, setNow] = useState<number | null>(null);
  useEffect(() => {
    setNow(Date.now());
    if (!ticking) return;
    const id = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(id);
  }, [ms, ticking]);
  return now;
}

const motionQuery = "(prefers-reduced-motion: reduce)";
function subscribeMotion(cb: () => void) {
  const mq = window.matchMedia(motionQuery);
  mq.addEventListener("change", cb);
  return () => mq.removeEventListener("change", cb);
}

/** True when the visitor asked for reduced motion. False on the server. */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(
    subscribeMotion,
    () => window.matchMedia(motionQuery).matches,
    () => false,
  );
}
