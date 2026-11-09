import Link from "next/link";
import { CheckItYourself } from "@/components/landing/CheckItYourself";
import { ClosingCta } from "@/components/landing/ClosingCta";
import { ForAgents } from "@/components/landing/ForAgents";
import { Hero, WatchBatchButton } from "@/components/landing/Hero";
import { HonestLimits } from "@/components/landing/HonestLimits";
import { HowItCrosses } from "@/components/landing/HowItCrosses";
import { RouteVsBook } from "@/components/landing/RouteVsBook";
import { SiteFooter } from "@/components/landing/SiteFooter";
import { SiteHeader } from "@/components/landing/SiteHeader";
import { fmtCompact, fmtDay, fmtPrice } from "@/lib/content/format";
import { LINKS, NETWORK, SETTLEMENT_RUNS } from "@/lib/content/proof";
import { loadBaseline, loadBatchProof, routeEdge } from "@/lib/content/results";
import "@/styles/landing.css";

export default function Home() {
  const baseline = loadBaseline();
  const proof = loadBatchProof();
  const { largest, refused } = routeEdge(baseline.rows.sell);

  const marquee = [
    `${proof.txs.crank} · batch ${proof.id} · two traders crossed at mid ${fmtPrice(proof.mid, 5)} · both legs · 1 tx`,
    ...SETTLEMENT_RUNS.map((r) => `${r.tx} · ${r.note}`),
  ];

  return (
    <div className="landing">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <SiteHeader overlay />
      <main id="main">
        <Hero marquee={marquee} marqueeLabel={`Public settlement artefacts, ${NETWORK.label}`}>
          <h1 className="h1" id="hero-title">
            Your order rests where nobody can read it.
          </h1>
          <p className="sub">
            Not other traders, not solvers, not us. Orders are sealed to an enclave, and even a compromised one can&apos;t
            fill you worse than the limit you signed.
          </p>
          <div className="ctas">
            <Link className="bb-btn" href={LINKS.app}>
              Seal a block
            </Link>
            <WatchBatchButton>Watch a batch settle</WatchBatchButton>
          </div>
          <p className="proof">
            Measured {fmtDay(baseline.measuredAt)} on the live 1Click route · selling {fmtCompact(largest.near)} NEAR:{" "}
            {Math.round(largest.costBps)} bps
            {refused ? ` · ${fmtCompact(refused.near)} NEAR: no liquidity` : null}
          </p>
        </Hero>
        <HowItCrosses proof={proof} />
        <RouteVsBook baseline={baseline} proof={proof} />
        <CheckItYourself proof={proof} />
        <ForAgents />
        <HonestLimits />
        <ClosingCta />
      </main>
      <SiteFooter />
    </div>
  );
}
