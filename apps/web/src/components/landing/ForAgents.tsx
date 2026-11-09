import type { ReactNode } from "react";
import { NETWORK } from "@/lib/content/proof";
import { CopyButton } from "./CopyButton";

const TOOLS: { name: string; does: string; noKey?: boolean }[] = [
  { name: "blindbook_markets", does: "Markets, lot sizes and the contracts it settles through", noKey: true },
  { name: "blindbook_route_cost", does: "What the block would cost on the public route right now", noKey: true },
  { name: "blindbook_batches", does: "Recent batches: matched lots, volumes, the mid, settlement", noKey: true },
  { name: "blindbook_balance", does: "Your balances inside the NEAR Intents verifier" },
  { name: "blindbook_place_order", does: "Signs lots at your limit, seals them locally, submits" },
  { name: "blindbook_order_status", does: "Lots still open versus filled or cancelled" },
  { name: "blindbook_cancel_order", does: "Cancel tickets straight to the verifier" },
];

/** The MCP client config, as shown and as copied. Matches packages/mcp/src/index.ts. */
const CONFIG = {
  mcpServers: {
    blindbook: {
      command: "node",
      args: ["blindbook/packages/mcp/dist/index.js"],
      env: {
        BLINDBOOK_NETWORK: NETWORK.id,
        BLINDBOOK_SECRET_KEY: "ed25519:…",
        BLINDBOOK_MAX_LOTS: "10",
      },
    },
  },
};

type Json = string | Json[] | { [k: string]: Json };

/** JSON with two-space indents, arrays of strings kept on one line. */
function toText(v: Json, depth = 0): string {
  const pad = "  ".repeat(depth);
  if (typeof v === "string") return JSON.stringify(v);
  if (Array.isArray(v)) return `[${v.map((x) => toText(x, depth)).join(", ")}]`;
  const rows = Object.entries(v).map(([k, x]) => `${pad}  ${JSON.stringify(k)}: ${toText(x, depth + 1)}`);
  return `{\n${rows.join(",\n")}\n${pad}}`;
}

/** The same layout as toText, as highlighted spans. */
function toSpans(v: Json, depth = 0): ReactNode {
  const pad = "  ".repeat(depth);
  const p = (t: string) => <span className="tp">{t}</span>;
  if (typeof v === "string") return <span className="ts">{JSON.stringify(v)}</span>;
  if (Array.isArray(v)) {
    return (
      <>
        {p("[")}
        {v.map((x, i) => (
          <span key={i}>
            {i > 0 ? p(", ") : null}
            {toSpans(x, depth)}
          </span>
        ))}
        {p("]")}
      </>
    );
  }
  const entries = Object.entries(v);
  return (
    <>
      {p("{")}
      {"\n"}
      {entries.map(([k, x], i) => (
        <span key={k}>
          {pad}
          {"  "}
          <span className="tk">{JSON.stringify(k)}</span>
          {p(":")} {toSpans(x, depth + 1)}
          {i < entries.length - 1 ? p(",") : null}
          {"\n"}
        </span>
      ))}
      {pad}
      {p("}")}
    </>
  );
}

export function ForAgents() {
  return (
    <section className="sec" id="agents" aria-labelledby="agents-h">
      <div className="bb-wrap agents">
        <div className="agents-copy">
          <h2 className="h2" id="agents-h">
            For agents and treasuries
          </h2>
          <p className="lead">
            Give your agent a mandate and seven tools. It prices the block against the public route, seals lots at its
            limit, watches them fill, and cancels without asking anyone.
          </p>
          <ul className="tools">
            {TOOLS.map((t) => (
              <li key={t.name}>
                <code>{t.name}</code>
                <span>{t.does}</span>
                {t.noKey ? <em>no key</em> : null}
              </li>
            ))}
          </ul>
        </div>

        <div className="code-card">
          <div className="code-top">
            <span className="code-file">mcp.json</span>
            <CopyButton text={toText(CONFIG)} label="Copy" />
          </div>
          <pre>
            <code>{toSpans(CONFIG)}</code>
          </pre>
          <ul className="code-notes">
            <li>Read-only tools work with no key at all.</li>
            <li>The agent&apos;s key never leaves this process. Orders are sealed locally before they&apos;re submitted.</li>
            <li>
              <code>BLINDBOOK_MAX_LOTS</code> is a hard cap on every order.
            </li>
          </ul>
        </div>
      </div>
    </section>
  );
}
