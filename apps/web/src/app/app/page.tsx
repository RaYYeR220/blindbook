import type { Metadata } from "next";
import { TradingApp } from "@/components/app/TradingApp";
import { SiteFooter } from "@/components/landing/SiteFooter";
import { SiteHeader } from "@/components/landing/SiteHeader";
import type { RouteCurve } from "@/lib/app/types";
import { loadBaseline } from "@/lib/content/results";
import "@/styles/app.css";

export const metadata: Metadata = {
  title: "App",
  description:
    "Seal a block at your own limit, watch a batch cross at the verified midpoint, and re-check every batch from public sources.",
};

/** The measured route curve ships with the page; everything live is read by the client from /api. */
function routeCurve(): RouteCurve {
  const b = loadBaseline();
  const rows = (side: "sell" | "buy") => b.rows[side].map((r) => ({ near: r.near, bps: r.costBps }));
  return { measuredAt: b.measuredAt, mid: b.mid, rows: { sell: rows("sell"), buy: rows("buy") } };
}

export default function AppPage() {
  return (
    <div className="app">
      <a className="skip-link" href="#app-main">
        Skip to the app
      </a>
      <SiteHeader />
      <main id="app-main">
        <h1 className="sr-only">Blindbook trading app</h1>
        <TradingApp curve={routeCurve()} />
      </main>
      <SiteFooter />
    </div>
  );
}
