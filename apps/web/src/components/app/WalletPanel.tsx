"use client";

import { useEffect, useState } from "react";
import { api } from "@/lib/app/api";
import { fmtUnits, shortAccount } from "@/lib/app/format";
import { accountUrl, IS_TESTNET, MARKET, NET } from "@/lib/app/net";
import type { FaucetResponse } from "@/lib/app/types";
import { connectEvm, connectNear, hasEvmWallet, needsKeyRegistration, walletErrorMessage } from "@/lib/app/wallet";
import { useApp } from "./AppContext";
import { Ext, PanelHead, TxLink } from "./ui";

export function WalletPanel() {
  const { wallet, walletReady, setWallet, balances, signingKey } = useApp();
  const [busy, setBusy] = useState<"near" | "evm" | "faucet" | "key" | "out" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [evm, setEvm] = useState(false);
  const [faucetOn, setFaucetOn] = useState(false);
  const [faucet, setFaucet] = useState<FaucetResponse | null>(null);
  const [keyTx, setKeyTx] = useState<string | null>(null);

  useEffect(() => {
    setEvm(hasEvmWallet());
    if (IS_TESTNET) {
      api
        .faucetInfo()
        .then((r) => setFaucetOn(r.available))
        .catch(() => setFaucetOn(false));
    }
  }, []);

  useEffect(() => {
    setFaucet(null);
    setKeyTx(null);
    setError(null);
  }, [wallet?.accountId]);

  const connect = async (kind: "near" | "evm") => {
    setBusy(kind);
    setError(null);
    try {
      setWallet(await (kind === "near" ? connectNear() : connectEvm()));
    } catch (e) {
      setError(walletErrorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async () => {
    if (!wallet) return;
    setBusy("out");
    await wallet.disconnect().catch(() => undefined);
    setWallet(null);
    setBusy(null);
  };

  const drip = async () => {
    if (!wallet) return;
    setBusy("faucet");
    setFaucet(await api.faucet(wallet.accountId));
    setBusy(null);
    void balances.refresh();
  };

  const register = async () => {
    if (!wallet?.registerKey || !signingKey) return;
    setBusy("key");
    setError(null);
    try {
      setKeyTx(await wallet.registerKey(signingKey));
      await balances.refresh();
    } catch (e) {
      setError(walletErrorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const bal = balances.data?.balances;
  const needsKey = wallet ? needsKeyRegistration(wallet) : false;
  const keyState = balances.data?.keyRegistered;

  return (
    <section className="panel wallet" aria-labelledby="wallet-h">
      <PanelHead id="wallet-h" title="Wallet" meta={IS_TESTNET ? "NEAR testnet" : "NEAR mainnet"} />
      {!wallet ? (
        <>
          <p className="muted">
            Sign lots with your own wallet. Nothing is held for you: each lot is a signed intent the verifier applies at your
            limit or not at all.
          </p>
          <div className="row-btns" role="group" aria-label="Connect a wallet">
            <button type="button" className="btn btn-line" disabled={!walletReady || busy !== null} onClick={() => connect("near")}>
              {busy === "near" ? "Opening…" : "NEAR wallet"}
            </button>
            <button type="button" className="btn btn-line" disabled={!walletReady || busy !== null || !evm} onClick={() => connect("evm")}>
              {busy === "evm" ? "Waiting…" : "EVM wallet"}
            </button>
          </div>
          {!evm && walletReady ? <p className="hint">No EVM wallet in this browser. NEAR wallets open in a picker.</p> : null}
        </>
      ) : (
        <>
          <div className="acct">
            <Ext href={accountUrl(wallet.accountId)} className="acct-id">
              <code title={wallet.accountId}>{shortAccount(wallet.accountId)}</code>
            </Ext>
            <span className="pill">{wallet.standard}</span>
            <button type="button" className="linkbtn" onClick={disconnect} disabled={busy === "out"}>
              Disconnect
            </button>
          </div>
          <dl className="bals" aria-busy={balances.loading && !bal}>
            <div>
              <dt>{MARKET.baseSymbol}</dt>
              <dd className="num">{bal ? fmtUnits(bal[MARKET.base] ?? "0", MARKET.baseDecimals, 2) : "–"}</dd>
            </div>
            <div>
              <dt>{MARKET.quoteSymbol}</dt>
              <dd className="num">{bal ? fmtUnits(bal[MARKET.quote] ?? "0", MARKET.quoteDecimals, 2) : "–"}</dd>
            </div>
            <div className="bals-src">
              <dt className="sr-only">Held in</dt>
              <dd>
                verifier balance on <code>{NET.verifier}</code>
              </dd>
            </div>
          </dl>
          {balances.error && !bal ? <p className="hint is-bad">Balances unavailable: {balances.error}</p> : null}

          {needsKey ? (
            keyState === true ? (
              <p className="keyok">
                <span aria-hidden="true">✓</span> Signing key registered on the verifier
                {keyTx ? <TxLink hash={keyTx} /> : null}
              </p>
            ) : keyState === false ? (
              <div className="stepbox" role="group" aria-labelledby="key-h">
                <p id="key-h" className="stepbox-t">
                  One-time step: register your signing key
                </p>
                <p className="stepbox-d">
                  A named account&apos;s key must be known to the verifier before its lots can settle. One{" "}
                  <code>add_public_key</code> call with 1 yoctoNEAR.
                </p>
                <button type="button" className="btn btn-sm" onClick={register} disabled={busy !== null}>
                  {busy === "key" ? "Waiting for wallet…" : "Register key"}
                </button>
              </div>
            ) : (
              <p className="hint">Your signing key is checked against the verifier at your first signature.</p>
            )
          ) : null}

          {IS_TESTNET && faucetOn ? (
            <div className="faucet">
              <button type="button" className="btn btn-line btn-sm" onClick={drip} disabled={busy !== null}>
                {busy === "faucet" ? "Crediting…" : "Get test tokens"}
              </button>
              <span className="hint">5 {MARKET.baseSymbol} and 20 {MARKET.quoteSymbol}, once a day</span>
              {faucet ? (
                faucet.ok ? (
                  <p className="faucet-ok" role="status">
                    Credited {fmtUnits(faucet.base, MARKET.baseDecimals, 0)} {MARKET.baseSymbol} and{" "}
                    {fmtUnits(faucet.quote, MARKET.quoteDecimals, 0)} {MARKET.quoteSymbol}
                    <TxLink hash={faucet.txs.base} />
                    <TxLink hash={faucet.txs.quote} />
                  </p>
                ) : (
                  <p className="hint is-bad" role="status">
                    {faucet.error}
                  </p>
                )
              ) : null}
            </div>
          ) : null}
        </>
      )}
      {error ? (
        <p className="hint is-bad" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
