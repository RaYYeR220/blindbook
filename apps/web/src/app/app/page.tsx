import type { Metadata } from "next";
import Link from "next/link";
import { SiteHeader } from "@/components/landing/SiteHeader";

export const metadata: Metadata = { title: "App" };

/** Placeholder until the trading app lands here. Built from the Tailwind token utilities. */
export default function AppPage() {
  return (
    <>
      <SiteHeader />
      <main className="mx-auto flex min-h-[70svh] max-w-wrap flex-col items-start justify-center px-gutter-sm md:px-gutter">
        <span className="mb-6 flex gap-1.5" aria-hidden="true">
          <i className="block h-[22px] w-9 rounded-tile border border-hair bg-tile" />
          <i className="block h-[22px] w-9 rounded-tile border border-hair-2" />
          <i className="block h-[22px] w-9 rounded-tile bg-lime" />
        </span>
        <h1 className="text-h2 font-semibold text-balance">The app loads here.</h1>
        <p className="mt-5 max-w-[560px] text-lead text-ink-2">
          Sealing, batches and fills will live on this page. Until then, the landing page walks through a real batch on
          testnet.
        </p>
        <Link
          href="/"
          className="mt-8 rounded-pill bg-ink px-[26px] py-3.5 font-medium text-ground transition-colors hover:bg-lime"
        >
          Back to the landing page
        </Link>
      </main>
    </>
  );
}
