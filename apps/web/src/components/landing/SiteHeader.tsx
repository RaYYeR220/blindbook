import Link from "next/link";
import { BrandMark } from "@/components/ui/BrandMark";
import { LINKS } from "@/lib/content/proof";

/**
 * Top bar. On the landing page it floats over the hero wall; on other pages
 * it sits in flow with a hairline under it.
 */
export function SiteHeader({ overlay = false, current }: { overlay?: boolean; current?: "protocol" }) {
  // section anchors live on the landing page
  const home = overlay ? "" : "/";
  return (
    <header className={`bb-header ${overlay ? "bb-reveal" : "bb-header--flow"}`}>
      <div className="bb-nav">
        <Link className="bb-brand" href="/" aria-label="Blindbook home">
          <BrandMark />
          Blindbook
        </Link>
        <nav className="bb-links" aria-label="Primary">
          <a href={`${home}#how`}>How the cross works</a>
          <a href={`${home}#check`}>Proof</a>
          <a href={`${home}#agents`}>Agents</a>
          <Link href={LINKS.protocol} aria-current={current === "protocol" ? "page" : undefined}>
            Docs
          </Link>
        </nav>
        <Link className="bb-nav-cta" href={LINKS.app}>
          Open the app
        </Link>
      </div>
    </header>
  );
}
