"use client";

import type { Side } from "@blindbook/sdk";
import { type Ref, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/app/api";
import { currentSalt, poolConfig } from "@/lib/app/chain";
import { fmtBps, fmtBytes, fmtPrice, fmtSignedBps, fmtUnits, fmtUsd, lotPrice, short } from "@/lib/app/format";
import { LOT_LABEL, LOT_UNITS, MARKET, MAX_LOTS, NET } from "@/lib/app/net";
import { ladderLimits, sealAndSubmit, type SealEvent } from "@/lib/app/orders";
import type { LotPlan, RouteCostResponse } from "@/lib/app/types";
import { needsKeyRegistration, walletErrorMessage } from "@/lib/app/wallet";
import { useApp } from "./AppContext";
import { BrokerBox } from "./BrokerBox";
import { PanelHead, Segmented, Tile, TxLink } from "./ui";

/** Default limit: 0.6% through the mid, so the lot crosses whenever the mid holds. */
const BAND = 0.006;
const TTLS = [10, 60, 360, 1440];
const ttlLabel = (m: number) => (m < 60 ? `${m} min` : m % 60 === 0 ? `${m / 60} h` : `${m} min`);
const PRICE_RE = /^\d+(\.\d{1,6})?$/;

interface SealState {
  state: "running" | "done" | "error";
  sign: { done: number; total: number };
  key?: "checking" | "registering" | "ok";
  keyTx?: string;
  encrypt?: { bytes: number; matcherKey: string };
  submit?: { state: "sending" | "ok"; tx?: string; orderId?: string | null };
  error?: string;
}

function reduce(s: SealState, e: SealEvent): SealState {
  switch (e.step) {
    case "sign":
      return { ...s, sign: { done: e.done, total: e.total } };
    case "key":
      return { ...s, key: e.state, keyTx: e.tx ?? s.keyTx };
    case "encrypt":
      return { ...s, encrypt: { bytes: e.bytes, matcherKey: e.matcherKey } };
    case "submit":
      return { ...s, submit: { state: e.state, tx: e.tx, orderId: e.orderId } };
  }
}

export function OrderTicket() {
  const { wallet, pool, mid, balances, setSigningKey, addOrder, refreshChain } = useApp();
  const [side, setSide] = useState<Side>("sell");
  const [lots, setLots] = useState(1);
  const [price, setPrice] = useState("");
  const [touched, setTouched] = useState(false);
  const [ladder, setLadder] = useState(0);
  const [minLots, setMinLots] = useState(1);
  const [ttl, setTtl] = useState(60);
  const [extraTtl, setExtraTtl] = useState<number | null>(null);
  const [planNote, setPlanNote] = useState<string | null>(null);
  const [seal, setSeal] = useState<SealState | null>(null);
  const [route, setRoute] = useState<{ data: RouteCostResponse | null; loading: boolean; error: string | null }>({
    data: null,
    loading: true,
    error: null,
  });

  const midPx = route.data?.mid ?? mid.data?.mid ?? null;

  // prefill the limit from the live mid until the trader types their own
  useEffect(() => {
    if (touched || midPx === null) return;
    setPrice((side === "sell" ? midPx * (1 - BAND) : midPx * (1 + BAND)).toFixed(4));
  }, [midPx, side, touched]);

  // the public route's cost for this exact size, from a live 1Click dry quote
  const size = lots * LOT_UNITS;
  useEffect(() => {
    const ctl = new AbortController();
    setRoute((r) => ({ ...r, loading: true }));
    const t = window.setTimeout(() => {
      api
        .routeCost(side, size, ctl.signal)
        .then((data) => setRoute({ data, loading: false, error: null }))
        .catch((e: Error) => {
          if (e.name !== "AbortError") setRoute((r) => ({ data: r.data, loading: false, error: e.message }));
        });
    }, 350);
    return () => {
      window.clearTimeout(t);
      ctl.abort();
    };
  }, [side, size]);

  useEffect(() => {
    if (minLots > lots) setMinLots(lots);
  }, [lots, minLots]);

  const validPrice = PRICE_RE.test(price.trim()) && Number(price) > 0;
  const limits = useMemo(() => {
    if (!validPrice) return null;
    try {
      return ladderLimits(MARKET, side, price.trim(), lots, ladder);
    } catch {
      return null;
    }
  }, [validPrice, side, price, lots, ladder]);

  const quoteTotal = limits?.reduce((a, b) => a + b, 0n) ?? 0n;
  const worthAtMid = midPx !== null ? size * midPx : null;
  /** the limit's distance from the mid, signed as prices are: a sell below it is negative, a buy above it positive */
  const offset = midPx !== null && validPrice ? ((Number(price) - midPx) / midPx) * 1e4 : null;
  const routeBps = route.data?.route_cost_bps ?? null;
  const routeUsd = routeBps !== null && worthAtMid !== null ? (routeBps / 1e4) * worthAtMid : null;
  const wontCross = offset !== null && (side === "sell" ? offset > 0 : offset < 0);

  const bal = balances.data?.balances;
  const have = bal ? BigInt(side === "sell" ? (bal[MARKET.base] ?? "0") : (bal[MARKET.quote] ?? "0")) : null;
  const need = side === "sell" ? BigInt(lots) * MARKET.lot : quoteTotal;
  const short_ = have !== null && have < need;
  const keyMissing = wallet && needsKeyRegistration(wallet) && balances.data?.keyRegistered === false;
  const matcherKey = pool.data?.config.matcher_key ?? null;
  const running = seal?.state === "running";

  const blocker = !wallet
    ? "Connect a wallet to seal lots."
    : !limits
      ? "Enter a limit price, e.g. 3.6100."
      : !matcherKey
        ? "The pool has not published its enclave key yet."
        : short_
          ? `Not enough ${side === "sell" ? MARKET.baseSymbol : MARKET.quoteSymbol}: need ${
              side === "sell" ? fmtUnits(need, MARKET.baseDecimals, 2) : fmtUnits(need, MARKET.quoteDecimals, 2)
            }, have ${side === "sell" ? fmtUnits(have ?? 0n, MARKET.baseDecimals, 2) : fmtUnits(have ?? 0n, MARKET.quoteDecimals, 2)}.`
          : keyMissing
            ? "Register your signing key first (wallet panel)."
            : null;

  const onPlan = (p: LotPlan) => {
    setSide(p.side);
    setLots(Math.min(MAX_LOTS, Math.max(1, p.lots)));
    setPrice(p.limit_price);
    setTouched(true);
    setLadder(p.ladder_step_bps);
    setMinLots(Math.max(1, Math.min(p.lots, p.min_lots)));
    if (!TTLS.includes(p.ttl_minutes)) setExtraTtl(p.ttl_minutes);
    setTtl(p.ttl_minutes);
    setPlanNote(p.rationale || "Filled from the broker's plan.");
    setSeal(null);
  };

  const submitRef = useRef<HTMLDivElement>(null);
  const sealNow = async () => {
    if (!wallet || !limits || !matcherKey || blocker) return;
    setSeal({ state: "running", sign: { done: 0, total: limits.length } });
    try {
      const [salt, cfg] = await Promise.all([currentSalt(), poolConfig().catch(() => pool.data?.config ?? null)]);
      const order = await sealAndSubmit({
        market: MARKET,
        verifier: NET.verifier,
        account: wallet.accountId,
        kind: wallet.kind,
        side,
        limits,
        minLots,
        ttlMinutes: ttl,
        salt,
        matcherKey,
        nextBatchId: cfg?.next_batch_id ?? 0,
        sign: (p) => wallet.sign(p),
        ensureKey: needsKeyRegistration(wallet)
          ? async (pk) => {
              setSigningKey(pk);
              const r = await api.balances(wallet.accountId, pk);
              if (r.keyRegistered) return;
              if (!wallet.registerKey) throw new Error("this wallet cannot register a key on the verifier");
              setSeal((s) => (s ? { ...s, key: "registering" } : s));
              const tx = await wallet.registerKey(pk);
              setSeal((s) => (s ? { ...s, keyTx: tx } : s));
            }
          : undefined,
        submit: api.submit,
        onEvent: (e) => setSeal((s) => (s ? reduce(s, e) : s)),
      });
      addOrder(order);
      setSeal((s) => (s ? { ...s, state: "done" } : s));
      refreshChain();
      void balances.refresh();
    } catch (e) {
      setSeal((s) => (s ? { ...s, state: "error", error: walletErrorMessage(e) } : s));
    }
    submitRef.current?.focus({ preventScroll: true });
  };

  const ttlOptions = [...TTLS, ...(extraTtl && !TTLS.includes(extraTtl) ? [extraTtl] : [])]
    .sort((a, b) => a - b)
    .map((m) => ({ value: String(m), label: ttlLabel(m) }));

  return (
    <section className="panel ticket" aria-labelledby="ticket-h">
      <PanelHead id="ticket-h" title="Seal a block" meta={<span className="num">lot {LOT_LABEL}</span>} />

      <BrokerBox onPlan={onPlan} mid={midPx} />

      <form
        className="tk"
        onSubmit={(e) => {
          e.preventDefault();
          void sealNow();
        }}
      >
        <fieldset className="tk-f" disabled={running}>
          <legend className="sr-only">Order</legend>
          <div className="fld">
            <span className="fld-l" id="side-l">
              Side
            </span>
            <Segmented<Side>
              label="Side"
              value={side}
              onChange={(v) => {
                setSide(v);
                setSeal(null);
              }}
              options={[
                { value: "sell", label: `Sell ${MARKET.baseSymbol}` },
                { value: "buy", label: `Buy ${MARKET.baseSymbol}` },
              ]}
            />
          </div>

          <div className="fld">
            <label className="fld-l" htmlFor="tk-lots">
              Lots <span className="fld-h">1–{MAX_LOTS}, {LOT_LABEL} each</span>
            </label>
            <div className="lots">
              <div className="stepper">
                <button type="button" aria-label="One lot fewer" onClick={() => setLots((n) => Math.max(1, n - 1))} disabled={lots <= 1}>
                  −
                </button>
                <input
                  id="tk-lots"
                  className="num"
                  type="number"
                  inputMode="numeric"
                  min={1}
                  max={MAX_LOTS}
                  value={lots}
                  onChange={(e) => setLots(Math.min(MAX_LOTS, Math.max(1, Math.round(Number(e.target.value) || 1))))}
                />
                <button type="button" aria-label="One lot more" onClick={() => setLots((n) => Math.min(MAX_LOTS, n + 1))} disabled={lots >= MAX_LOTS}>
                  +
                </button>
              </div>
              <span className="lot-row" aria-hidden="true">
                {Array.from({ length: MAX_LOTS }, (_, i) => (
                  <Tile key={i} kind={i < lots ? "sealed" : "void"} small />
                ))}
              </span>
            </div>
          </div>

          <div className="fld">
            <label className="fld-l" htmlFor="tk-px">
              Limit <span className="fld-h">{MARKET.quoteSymbol} per {MARKET.baseSymbol}, {side === "sell" ? "lowest you accept" : "highest you pay"}</span>
            </label>
            <div className="px">
              <input
                id="tk-px"
                className="input num"
                inputMode="decimal"
                autoComplete="off"
                spellCheck={false}
                value={price}
                aria-invalid={price !== "" && !validPrice}
                aria-describedby="tk-px-h"
                onChange={(e) => {
                  setPrice(e.target.value.replace(",", "."));
                  setTouched(true);
                  setSeal(null);
                }}
                placeholder={midPx ? fmtPrice(midPx, 4) : "3.6000"}
              />
              {touched && midPx !== null ? (
                <button type="button" className="linkbtn" onClick={() => setTouched(false)}>
                  mid {side === "sell" ? "−" : "+"}0.6%
                </button>
              ) : null}
            </div>
            <p className="fld-x num" id="tk-px-h">
              {midPx !== null ? `mid ${fmtPrice(midPx, 4)}` : "mid loading…"}
              {offset !== null ? ` · limit ${fmtSignedBps(offset)} from mid` : ""}
            </p>
            {wontCross ? (
              <p className="hint is-warn">
                This limit is better than the mid, so the lot rests until the mid reaches it.
              </p>
            ) : null}
          </div>

          <div className="fld-2">
            <div className="fld">
              <label className="fld-l" htmlFor="tk-ladder">
                Ladder <span className="fld-h">bps per lot</span>
              </label>
              <input
                id="tk-ladder"
                className="input num"
                type="number"
                inputMode="numeric"
                min={0}
                max={100}
                value={ladder}
                onChange={(e) => setLadder(Math.min(100, Math.max(0, Math.round(Number(e.target.value) || 0))))}
              />
            </div>
            <div className="fld">
              <label className="fld-l" htmlFor="tk-min">
                Min lots <span className="fld-h">per cross</span>
              </label>
              <input
                id="tk-min"
                className="input num"
                type="number"
                inputMode="numeric"
                min={1}
                max={lots}
                value={minLots}
                onChange={(e) => setMinLots(Math.min(lots, Math.max(1, Math.round(Number(e.target.value) || 1))))}
              />
            </div>
          </div>
          {ladder > 0 && limits && limits.length > 1 ? (
            <p className="fld-x num">
              lots at {limits.map((l) => fmtPrice(lotPrice(MARKET, l), 4)).join(" · ")}
            </p>
          ) : null}

          <div className="fld">
            <span className="fld-l">Valid for</span>
            <Segmented<string> label="Valid for" size="sm" value={String(ttl)} onChange={(v) => setTtl(Number(v))} options={ttlOptions} />
          </div>
        </fieldset>

        {planNote ? (
          <p className="plan-note">
            <span className="pill is-lime">plan</span> {planNote} Edit anything before you sign.
          </p>
        ) : null}

        <dl className="sum">
          <div>
            <dt>Worth at mid</dt>
            <dd className="num">{worthAtMid !== null ? fmtUsd(worthAtMid) : "–"}</dd>
          </div>
          <div>
            <dt>Public route, same size</dt>
            <dd className="num" aria-busy={route.loading}>
              {routeBps !== null ? (
                <>
                  −{fmtBps(routeBps)} <span className="sum-x">{routeUsd !== null ? fmtUsd(routeUsd) : ""}</span>
                </>
              ) : route.data ? (
                <span className="sum-x">{route.data.route.replace(/^unavailable:\s*/, "no route: ")}</span>
              ) : route.error ? (
                <span className="sum-x">unavailable</span>
              ) : (
                "…"
              )}
            </dd>
          </div>
          <div className="is-pin">
            <dt>Worst case: your own limit</dt>
            <dd className="num">
              {limits
                ? `${side === "sell" ? "receive ≥" : "pay ≤"} ${fmtUnits(quoteTotal, MARKET.quoteDecimals, 2)} ${MARKET.quoteSymbol}`
                : "–"}
            </dd>
          </div>
        </dl>

        <button type="submit" className="btn btn-seal" disabled={running || blocker !== null} aria-describedby="tk-why">
          {running ? "Sealing…" : `Seal ${lots} ${lots === 1 ? "lot" : "lots"}`}
        </button>
        <p className="hint" id="tk-why">
          {blocker ?? `${lots} wallet ${lots === 1 ? "prompt" : "prompts"}, one per lot. The relay pays the gas.`}
        </p>
      </form>

      {seal ? <SealProgress seal={seal} ref={submitRef} /> : null}
    </section>
  );
}

function SealProgress({ seal, ref }: { seal: SealState; ref: Ref<HTMLDivElement> }) {
  const signed = seal.sign.done === seal.sign.total;
  const st = (on: boolean, now: boolean) => (on ? "done" : now ? (seal.state === "error" ? "fail" : "now") : "todo");
  const signState = st(signed, !signed);
  const encState = st(Boolean(seal.encrypt), signed && !seal.encrypt);
  const subState = st(seal.submit?.state === "ok", Boolean(seal.encrypt) && seal.submit?.state !== "ok");
  return (
    <div className="seal" ref={ref} tabIndex={-1} aria-labelledby="seal-h">
      <p className="sr-only" id="seal-h">
        Sealing progress
      </p>
      <ol className="steps is-tight" aria-live="polite">
        <li className={`step is-${signState}`}>
          <span className="step-i num" aria-hidden="true">
            {signState === "done" ? "✓" : 1}
          </span>
          <span className="step-l">
            Sign {seal.sign.total} {seal.sign.total === 1 ? "lot" : "lots"}{" "}
            <span className="num">
              {seal.sign.done}/{seal.sign.total}
            </span>
          </span>
          <span className="step-n">
            {seal.key === "registering"
              ? "registering your key on the verifier (one time)…"
              : seal.keyTx
                ? "key registered"
                : signed
                  ? "each lot is a token_diff at its own limit"
                  : "approve each lot in your wallet"}
            {seal.keyTx ? <TxLink hash={seal.keyTx} /> : null}
          </span>
        </li>
        <li className={`step is-${encState}`}>
          <span className="step-i num" aria-hidden="true">
            {encState === "done" ? "✓" : 2}
          </span>
          <span className="step-l">Encrypt to the enclave key</span>
          <span className="step-n num">
            {seal.encrypt
              ? `key ${short(seal.encrypt.matcherKey, 8, 8)} · ${fmtBytes(seal.encrypt.bytes)}, padded to a fixed ${Math.round((seal.encrypt.bytes - 73) / 1024)} KiB bucket`
              : "X25519 + XChaCha20-Poly1305, padded to a fixed size"}
          </span>
        </li>
        <li className={`step is-${subState}`}>
          <span className="step-i num" aria-hidden="true">
            {subState === "done" ? "✓" : 3}
          </span>
          <span className="step-l">Submit through the relay</span>
          <span className="step-n">
            {seal.submit?.state === "ok" ? (
              <>
                order <span className="num">#{seal.submit.orderId ?? "?"}</span> stored in the pool{" "}
                {seal.submit.tx ? <TxLink hash={seal.submit.tx} /> : null}
              </>
            ) : (
              "the relayer pays gas; it can't read or change the order"
            )}
          </span>
        </li>
      </ol>
      {seal.state === "error" ? (
        <p className="hint is-bad" role="alert">
          {seal.error}
        </p>
      ) : null}
      {seal.state === "done" ? (
        <p className="seal-ok" role="status">
          Sealed. It rests where nobody can read it until a batch crosses it at the mid.
        </p>
      ) : null}
    </div>
  );
}
