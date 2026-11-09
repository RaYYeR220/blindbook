import Link from "next/link";
import { BrandMark } from "@/components/ui/BrandMark";
import { LINKS, NETWORK } from "@/lib/content/proof";

export function SiteFooter() {
  return (
    <footer className="bb-footer">
      <div className="bb-wrap bb-footer-in">
        <div className="bb-footer-brand">
          <Link className="bb-brand" href="/" aria-label="Blindbook home">
            <BrandMark />
            Blindbook
          </Link>
          <p>A dark pool for NEAR Intents. Live on {NETWORK.id}.</p>
        </div>
        <nav className="bb-footer-links" aria-label="Footer">
          <a href={LINKS.github} rel="noopener">
            GitHub
          </a>
          <Link href={LINKS.protocol}>Docs</Link>
          <a href={LINKS.x}>X</a>
        </nav>
      </div>
    </footer>
  );
}
