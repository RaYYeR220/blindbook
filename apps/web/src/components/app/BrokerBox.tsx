"use client";

import { useEffect, useId, useState } from "react";
import { api } from "@/lib/app/api";
import { LOT_LABEL, MARKET } from "@/lib/app/net";
import type { BrokerInfo, BrokerResponse, LotPlan } from "@/lib/app/types";
import { useNow } from "./hooks";
import { CheckList } from "./ui";

/** The broker's evidence, named for what each check proves. */
const EVIDENCE: Record<string, string> = {
  tdx: "TDX quote bound to a fresh nonce",
  gpu: "NVIDIA GPU attestation",
  signature: "Answer signed by the attested key",
};

/**
 * "Describe your block": a plain-language mandate goes to a model running in
 * a TEE, and comes back as a lot plan that fills the ticket. Nothing is
 * signed until the trader reviews the ticket and seals it.
 */
export function BrokerBox({ onPlan, mid }: { onPlan: (p: LotPlan) => void; mid: number | null }) {
  const [info, setInfo] = useState<BrokerInfo | null>(null);
  const [mandate, setMandate] = useState("");
  const [running, setRunning] = useState<number | null>(null);
  const [result, setResult] = useState<BrokerResponse | null>(null);
  const now = useNow(1000, running !== null);
  const id = useId();

  useEffect(() => {
    api
      .brokerInfo()
      .then(setInfo)
      .catch(() => setInfo({ available: false, model: "" }));
  }, []);

  const online = info?.available === true;
  const tooShort = mandate.trim().length < 6;

  const ask = async () => {
    if (!online || tooShort || running) return;
    setRunning(Date.now());
    setResult(null);
    const r = await api.broker({
      mandate: mandate.trim(),
      mid: mid ?? 0,
      lot: LOT_LABEL,
      market: `${MARKET.baseSymbol}/${MARKET.quoteSymbol}`,
    });
    setRunning(null);
    setResult(r);
    if (r.ok) onPlan(r.plan);
  };

  const waited = running && now ? Math.max(0, Math.floor((now - running) / 1000)) : 0;

  return (
    <div className={`broker${online ? "" : " is-off"}`}>
      <div className="broker-h">
        <label htmlFor={`${id}-m`} className="broker-t">
          Describe your block
        </label>
        <span className={`pill${online ? " is-lime" : ""}`}>
          {info === null ? "checking broker…" : online ? "private AI broker" : "private AI broker: coming online"}
        </span>
      </div>
      <textarea
        id={`${id}-m`}
        className="input broker-in"
        rows={2}
        maxLength={600}
        disabled={!online || running !== null}
        value={mandate}
        onChange={(e) => setMandate(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void ask();
        }}
        placeholder={`Sell 6 ${MARKET.baseSymbol} today, never below 3.55, in a gentle ladder.`}
        aria-describedby={`${id}-h`}
      />
      <div className="broker-row">
        <button type="button" className="btn btn-line btn-sm" disabled={!online || tooShort || running !== null} onClick={ask}>
          {running ? `Planning… ${waited} s` : "Plan it privately"}
        </button>
        <span className="hint" id={`${id}-h`}>
          {online
            ? `${info?.model} in a TEE. Your mandate is never stored; the plan fills the ticket for you to edit.`
            : "The mandate box opens once the attested model is configured. The ticket works without it."}
        </span>
      </div>
      <div aria-live="polite">
        {running ? <p className="hint">Attesting the model with a fresh nonce, then asking it. Up to 20 s.</p> : null}
        {result && !result.ok ? (
          <p className="hint is-bad" role="alert">
            {result.error}
          </p>
        ) : null}
        {result?.ok ? (
          <p className="broker-plan">
            Plan: {result.plan.side} {result.plan.lots} {result.plan.lots === 1 ? "lot" : "lots"} at{" "}
            <span className="num">{result.plan.limit_price}</span>
            {result.plan.ladder_step_bps > 0 ? (
              <>
                , ladder <span className="num">{result.plan.ladder_step_bps}</span> bps
              </>
            ) : null}
            . Ticket filled below.
          </p>
        ) : null}
      </div>
      {result?.checks && result.checks.length > 0 ? (
        <div className="broker-ev">
          <p className="broker-ev-h">Private model evidence</p>
          <CheckList checks={result.checks} labels={EVIDENCE} stagger />
        </div>
      ) : null}
    </div>
  );
}
