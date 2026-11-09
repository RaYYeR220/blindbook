"use client";

import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { TileCheck } from "@/components/ui/BrandMark";
import { short } from "@/lib/app/format";
import { txUrl } from "@/lib/app/net";
import type { Check } from "@/lib/app/types";
import { useReducedMotion } from "./hooks";

/** One 36 × 22 lot tile in the landing's language: sealed, crossed (lime with a check), open, or refused. */
export type TileKind = "sealed" | "crossed" | "open" | "void" | "wait" | "fail" | "refused";

export function Tile({ kind, small = false, label }: { kind: TileKind; small?: boolean; label?: string }) {
  return (
    <span className={`tile is-${kind}${small ? " is-sm" : ""}`} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      {kind === "crossed" ? <TileCheck /> : null}
      {kind === "fail" || kind === "refused" ? (
        <svg viewBox="0 0 36 22" aria-hidden="true" focusable="false">
          <path d="M14 7l8 8M22 7l-8 8" />
        </svg>
      ) : null}
    </span>
  );
}

const ext = { target: "_blank", rel: "noopener noreferrer" } as const;

/** A transaction hash, shortened, linking to the explorer. */
export function TxLink({ hash, label }: { hash: string; label?: string }) {
  return (
    <a className="txl" href={txUrl(hash)} {...ext} title={hash}>
      {label ? <span className="txl-k">{label}</span> : null}
      <code>{short(hash, 6, 4)}</code>
      <span aria-hidden="true">↗</span>
      <span className="sr-only"> (opens the explorer)</span>
    </a>
  );
}

/** An external link that says so to screen readers. */
export function Ext({ href, children, className }: { href: string; children: ReactNode; className?: string }) {
  return (
    <a href={href} className={className} {...ext}>
      {children}
      <span className="sr-only"> (opens in a new tab)</span>
    </a>
  );
}

/**
 * The verification checks, one row each. With `stagger`, rows reveal one
 * after another so a fresh result reads as checks turning green.
 */
export function CheckList({
  checks,
  labels,
  stagger = false,
  pending = 0,
}: {
  checks: Check[];
  labels?: Record<string, string>;
  stagger?: boolean;
  /** placeholder rows while the checks are running */
  pending?: number;
}) {
  const reduced = useReducedMotion();
  const [shown, setShown] = useState(stagger && !reduced ? 0 : checks.length);
  const key = checks.map((c) => `${c.id}:${c.ok}`).join(",");
  useEffect(() => {
    if (!stagger || reduced) {
      setShown(checks.length);
      return;
    }
    setShown(0);
    let i = 0;
    const id = window.setInterval(() => {
      i += 1;
      setShown(i);
      if (i >= checks.length) window.clearInterval(id);
    }, 170);
    return () => window.clearInterval(id);
    // re-run only when the set of results changes
  }, [key, stagger, reduced, checks.length]);

  if (checks.length === 0 && pending > 0) {
    return (
      <ol className="cks" aria-busy="true">
        {Array.from({ length: pending }, (_, i) => (
          <li key={i} className="ck is-wait">
            <Tile kind="wait" small />
            <span className="ck-l">
              <span className="sk" style={{ width: `${48 + ((i * 37) % 40)}%` }} />
            </span>
          </li>
        ))}
      </ol>
    );
  }

  return (
    <ol className="cks">
      {checks.map((c, i) => {
        const on = i < shown;
        const kind: TileKind = !on ? "wait" : c.ok === true ? "crossed" : c.ok === false ? "fail" : "open";
        const verdict = c.ok === true ? "passes" : c.ok === false ? "fails" : "not applicable";
        return (
          <li key={c.id} className={`ck${on ? " is-on" : ""}`} data-ok={on ? String(c.ok) : undefined}>
            <Tile kind={kind} small />
            <div className="ck-l">
              <span className="ck-t" title={c.label}>
                {labels?.[c.id] ?? c.label}
                <span className="sr-only">: {verdict}</span>
              </span>
              <span className="ck-d">{c.detail}</span>
            </div>
            {c.link ? (
              <Ext href={c.link} className="ck-a">
                open
              </Ext>
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}

/** A radio group drawn as the landing's segmented control, with arrow-key movement. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled,
  size = "md",
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
  disabled?: boolean;
  size?: "md" | "sm";
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const i = (options.findIndex((o) => o.value === value) + step + options.length) % options.length;
    onChange(options[i].value);
    refs.current[i]?.focus();
  };
  return (
    <div className={`bb-seg seg is-${size}`} role="radiogroup" aria-label={label} onKeyDown={onKey}>
      {options.map((o, i) => (
        <button
          key={o.value}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          tabIndex={value === o.value ? 0 : -1}
          disabled={disabled}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** A panel heading with an optional meta line on the right. */
export function PanelHead({ id, title, meta, children }: { id?: string; title: ReactNode; meta?: ReactNode; children?: ReactNode }) {
  return (
    <header className="ph">
      <h2 className="ph-t" id={id}>
        {title}
      </h2>
      {meta ? <span className="ph-m">{meta}</span> : null}
      {children}
    </header>
  );
}
