"use client";

import type { Side } from "@blindbook/sdk";
import { createContext, type ReactNode, use, useCallback, useEffect, useMemo, useState } from "react";
import { api } from "@/lib/app/api";
import { NET } from "@/lib/app/net";
import { loadOrders, type StoredOrder, saveOrders } from "@/lib/app/orders";
import { poolConfig } from "@/lib/app/chain";
import type { BalancesResponse, BatchView, PoolConfigView, PoolResponse, RouteCostResponse } from "@/lib/app/types";
import { onEvmAccountChange, restoreWallet, type Wallet } from "@/lib/app/wallet";
import { type Polled, usePoll } from "./hooks";

/** The batch the attestation rail is showing, and why. */
export interface Focus {
  batchId: string;
  /** the crank transaction, when the page already knows it */
  tx?: string | null;
  source: "demo" | "picked" | "crank";
}

/** One fill to analyse against the public route. */
export interface Fill {
  side: Side;
  lots: number;
  /** the batch's verified midpoint */
  mid: number;
  /** what each lot settled at */
  price: number;
  /** the trader's own limit (the first lot's, for a ladder) */
  limit: number;
  batchId: string;
  source: "demo" | "order";
}

export type Pane = "trade" | "tape" | "proof";

interface AppValue {
  pool: Polled<PoolResponse>;
  /** the pool's config read straight from the chain, so a fresh order or crank shows within seconds */
  config: Polled<PoolConfigView>;
  batches: Polled<BatchView[]>;
  /** the live NEAR/USDC midpoint from two 1Click dry quotes */
  mid: Polled<RouteCostResponse>;
  wallet: Wallet | null;
  walletReady: boolean;
  setWallet: (w: Wallet | null) => void;
  /** verifier balances of the connected account, and whether its NEAR key is registered */
  balances: Polled<BalancesResponse>;
  /** the NEAR key that signs for the account, once known */
  signingKey: string | null;
  setSigningKey: (k: string | null) => void;
  /** this account's orders, as this browser remembers them */
  orders: StoredOrder[];
  addOrder: (o: StoredOrder) => void;
  updateOrder: (key: string, fn: (o: StoredOrder) => StoredOrder) => void;
  focus: Focus | null;
  setFocus: (f: Focus | null) => void;
  fill: Fill | null;
  setFill: (f: Fill | null) => void;
  pane: Pane;
  setPane: (p: Pane) => void;
  /** refresh pool and batches now, e.g. after a crank */
  refreshChain: () => void;
}

const Ctx = createContext<AppValue | null>(null);

export function useApp(): AppValue {
  const v = use(Ctx);
  if (!v) throw new Error("useApp needs <AppProvider>");
  return v;
}

export function AppProvider({ children }: { children: ReactNode }) {
  const pool = usePoll(api.pool, 30_000);
  const config = usePoll(poolConfig, 6_000);
  const batches = usePoll(() => api.batches(12), 10_000);
  const mid = usePoll(() => api.routeCost("sell", 1), 45_000);
  const [wallet, setWalletState] = useState<Wallet | null>(null);
  const [walletReady, setWalletReady] = useState(false);
  const [learnedKey, setSigningKey] = useState<string | null>(null);
  const signingKey = learnedKey ?? wallet?.publicKey ?? null;
  const account = wallet?.accountId ?? "";
  const balances = usePoll(() => api.balances(account, signingKey), 20_000, account !== "", `${account}|${signingKey ?? ""}`);
  const setWallet = useCallback((w: Wallet | null) => {
    setWalletState(w);
    setSigningKey(null);
  }, []);
  const [focus, setFocus] = useState<Focus | null>(null);
  const [fill, setFill] = useState<Fill | null>(null);
  const [pane, setPane] = useState<Pane>("trade");

  const [orders, setOrders] = useState<StoredOrder[]>([]);
  useEffect(() => {
    setOrders(account ? loadOrders(NET.networkId, account) : []);
  }, [account]);
  const commit = useCallback(
    (fn: (prev: StoredOrder[]) => StoredOrder[]) => {
      if (!account) return;
      setOrders((prev) => {
        const next = fn(prev);
        saveOrders(NET.networkId, account, next);
        return next;
      });
    },
    [account],
  );
  const addOrder = useCallback((o: StoredOrder) => commit((prev) => [o, ...prev]), [commit]);
  const updateOrder = useCallback(
    (key: string, fn: (o: StoredOrder) => StoredOrder) => commit((prev) => prev.map((o) => (o.key === key ? fn(o) : o))),
    [commit],
  );

  useEffect(() => {
    let alive = true;
    restoreWallet()
      .then((w) => {
        if (alive && w) setWalletState(w);
      })
      .finally(() => {
        if (alive) setWalletReady(true);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (wallet?.kind !== "evm") return;
    return onEvmAccountChange((w) => setWallet(w));
  }, [wallet?.kind, setWallet]);

  const { refresh: refreshPool } = pool;
  const { refresh: refreshConfig } = config;
  const { refresh: refreshBatches } = batches;
  const value = useMemo<AppValue>(
    () => ({
      pool,
      config,
      batches,
      mid,
      wallet,
      walletReady,
      setWallet,
      balances,
      signingKey,
      setSigningKey,
      orders,
      addOrder,
      updateOrder,
      focus,
      setFocus,
      fill,
      setFill,
      pane,
      setPane,
      refreshChain: () => {
        void refreshPool();
        void refreshConfig();
        void refreshBatches();
      },
    }),
    [
      pool,
      config,
      batches,
      mid,
      wallet,
      walletReady,
      setWallet,
      balances,
      signingKey,
      orders,
      addOrder,
      updateOrder,
      focus,
      fill,
      pane,
      refreshPool,
      refreshConfig,
      refreshBatches,
    ],
  );
  return <Ctx value={value}>{children}</Ctx>;
}
