import Link from "next/link";
import { LINKS } from "@/lib/content/proof";
import { EndWall } from "./EndWall";

export function ClosingCta() {
  return (
    <section className="end" aria-labelledby="end-h">
      <EndWall />
      <div className="end-panel">
        <h2 className="h2" id="end-h">
          Sell size without being&nbsp;seen.
        </h2>
        <div className="ctas">
          <Link className="bb-btn" href={LINKS.app}>
            Open the app
          </Link>
          <Link className="bb-link" href={LINKS.protocol}>
            Read the protocol
          </Link>
        </div>
      </div>
    </section>
  );
}
