"use client";

import { useEffect, useState } from "react";
import { CopyButton } from "@/components/landing/CopyButton";
import { getBatch } from "@/lib/app/chain";
import { short } from "@/lib/app/format";
import { accountUrl, attestationUrl, IS_TESTNET, NET, wasmUrl } from "@/lib/app/net";
import type { BatchView } from "@/lib/app/types";
import { CHECK_SHORT, type Verification, verifyBatch } from "@/lib/app/verify";
import { useApp } from "./AppContext";
import { latestCrossed } from "./BatchTape";
import { CheckList, Ext, PanelHead, TxLink } from "./ui";

const SOURCE: Record<string, string> = { demo: "your demo", picked: "picked", crank: "your crank" };

export function AttestationRail() {
  const { focus, batches, pool } = useApp();
  const rows = batches.data ?? [];
  const fallback = latestCrossed(rows);
  const targetId = focus?.batchId ?? fallback?.id ?? null;
  const fromTape = targetId ? rows.find((b) => b.id === targetId) : undefined;
  const [fetched, setFetched] = useState<BatchView | null>(null);
  const batch = fromTape ?? (fetched?.id === targetId ? fetched : null);

  // a batch just cranked may not be on the cached tape yet: read it from the chain
  useEffect(() => {
    if (!targetId || fromTape) return;
    let alive = true;
    const read = () =>
      getBatch(targetId)
        .then((b) => {
          if (alive && b) setFetched(b);
        })
        .catch(() => undefined);
    void read();
    const id = window.setInterval(read, 4000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [targetId, fromTape]);

  const [ver, setVer] = useState<{ key: string; v: Verification | null; error: string | null } | null>(null);
  const verKey = batch ? `${batch.id}:${batch.status}` : null;
  const tx = focus?.batchId === batch?.id ? (focus?.tx ?? null) : null;
  useEffect(() => {
    if (!batch || !verKey) return;
    let alive = true;
    setVer((v) => (v?.key === verKey ? v : { key: verKey, v: null, error: null }));
    verifyBatch(batch, tx)
      .then((v) => alive && setVer({ key: verKey, v, error: null }))
      .catch((e: Error) => alive && setVer({ key: verKey, v: null, error: e.message }));
    return () => {
      alive = false;
    };
    // re-verify only when the batch or its status changes
  }, [verKey]);

  const cfg = pool.data?.config;
  const v = ver?.key === verKey ? ver.v : null;
  const pass = v?.checks.filter((c) => c.ok === true).length ?? 0;
  const fail = v?.checks.filter((c) => c.ok === false).length ?? 0;
  const source = focus ? SOURCE[focus.source] : "latest settled";
  const cmd = batch
    ? `pnpm verify:batch ${batch.id}${v?.tx ? ` --tx ${v.tx} --sender ${v.sender}` : ""}${IS_TESTNET ? "" : " --mainnet"}`
    : "";
  const wasm = cfg ? wasmUrl(cfg.version_key) : null;

  return (
    <section className="panel attest" aria-labelledby="attest-h">
      <PanelHead
        id="attest-h"
        title="Attestation"
        meta={batch ? <span className="num">batch #{batch.id} · {source}</span> : null}
      />
      {!targetId ? (
        <p className="muted">{batches.loading ? "Finding the latest settled batch…" : "No settled batch to check yet."}</p>
      ) : (
        <>
          <p className="att-sum" aria-live="polite">
            {v ? (
              <>
                <b className={fail ? "bad" : "ok"}>
                  {pass} of {v.checks.length}
                </b>{" "}
                checks pass{fail ? `, ${fail} fail` : ""}
                {v.checks.length < 7 && !v.tx ? ". The crank tx was not found, so the enclave checks are skipped." : "."}
              </>
            ) : ver?.error ? (
              <span className="bad">Could not verify: {ver.error}</span>
            ) : (
              `Re-checking batch #${targetId} from public sources…`
            )}
          </p>
          <CheckList checks={v?.checks ?? []} labels={CHECK_SHORT} stagger={focus?.source === "demo"} pending={v || ver?.error ? 0 : 7} />
        </>
      )}

      <dl className="facts">
        <div>
          <dt>Pinned matcher build</dt>
          <dd>
            {cfg ? (
              wasm ? (
                <Ext href={wasm}>
                  <code title={cfg.version_key}>{short(cfg.version_key, 10, 6)}</code>
                </Ext>
              ) : (
                <code title={cfg.version_key}>{short(cfg.version_key, 10, 6)}</code>
              )
            ) : (
              "–"
            )}
            <span>sha256 of the wasm the enclave must run</span>
          </dd>
        </div>
        <div>
          <dt>Result key</dt>
          <dd>
            <code title={cfg?.result_key ?? ""}>{cfg?.result_key ? short(cfg.result_key, 14, 6) : "–"}</code>
            <span>the pool acts only on results it signed</span>
          </dd>
        </div>
        <div>
          <dt>Venue key</dt>
          <dd>
            <code title={cfg?.venue_key ?? ""}>{cfg?.venue_key ? short(cfg.venue_key, 14, 6) : "–"}</code>
            <span>
              held by the enclave, for{" "}
              {cfg ? (
                <Ext href={accountUrl(cfg.venue_account)}>
                  <code>{cfg.venue_account}</code>
                </Ext>
              ) : null}
            </span>
          </dd>
        </div>
        <div>
          <dt>OutLayer attestation</dt>
          <dd>
            {v?.request_id ? (
              <Ext href={attestationUrl(v.request_id)}>
                <code>request {v.request_id}</code>
              </Ext>
            ) : (
              <code>{v ? "not linked" : "–"}</code>
            )}
            <span>Intel TDX quote for this run</span>
          </dd>
        </div>
        {v?.tx ? (
          <div>
            <dt>Crank</dt>
            <dd>
              <TxLink hash={v.tx} />
              <span>run_batch, on_result and both legs</span>
            </dd>
          </div>
        ) : null}
        <div>
          <dt>Pool</dt>
          <dd>
            <Ext href={accountUrl(NET.pool)}>
              <code>{NET.pool}</code>
            </Ext>
          </dd>
        </div>
      </dl>

      {batch ? (
        <div className="rerun">
          <p className="rerun-h">Re-run it locally</p>
          <pre className="cmd">
            <code>{cmd}</code>
          </pre>
          <CopyButton text={cmd} label="Copy command" />
        </div>
      ) : null}
    </section>
  );
}
