import { fmtCompact, fmtDay, fmtDayYear, fmtInt, fmtTimeUtc, fmtUsd, fmtUsdK } from "@/lib/content/format";
import { BASELINE_FILE, NETWORK } from "@/lib/content/proof";
import { type Baseline, type BatchProof, type RouteRow, routeEdge, type Side } from "@/lib/content/results";
import { RouteChart, type RouteRowView, RouteSideProvider, SideToggle } from "./RouteChart";

function rowViews(base: Baseline, proof: BatchProof, bookBps: number): Record<Side, RouteRowView[]> {
  const view = (side: Side, rows: RouteRow[]): RouteRowView[] => {
    let cliffDone = false;
    const out = rows.map((r): RouteRowView => {
      const none = r.costBps == null || r.costUsd == null;
      const cliff = none && !cliffDone;
      if (none) cliffDone = true;
      const head = `1Click dry quote · ${side} ${fmtInt(r.near)} NEAR`;
      return {
        key: String(r.near),
        kind: none ? "none" : "route",
        bps: none ? 0 : Math.round(r.costBps!),
        cliff,
        size: `${fmtInt(r.near)} NEAR`,
        sizeNote: `${fmtUsd(r.usdAtMid)} at mid`,
        cost: none ? "no route" : `${Math.round(r.costBps!)} bps`,
        costNote: none ? null : fmtUsd(r.costUsd!),
        quote: r.unavailable,
        tip: none
          ? `${head} · “${r.unavailable}”`
          : `${head} · ${r.costBps!.toFixed(2)} bps · ${fmtUsd(r.costUsd!)} vs mid ${base.mid}`,
      };
    });
    out.push({
      key: "book",
      kind: "book",
      bps: bookBps,
      cliff: false,
      size: "Blindbook",
      sizeNote: "any size, when the other side is in the batch",
      cost: `≤${bookBps} bps`,
      costNote: "from the mid",
      quote: null,
      tip: `batch ${proof.id}, ${NETWORK.label} · seller mid − ${Math.round(proof.sellerBps)} bps · buyer mid + ${Math.round(proof.buyerBps)} bps`,
    });
    return out;
  };
  return { sell: view("sell", base.rows.sell), buy: view("buy", base.rows.buy) };
}

export function RouteVsBook({ baseline, proof }: { baseline: Baseline; proof: BatchProof }) {
  const { largest, refused } = routeEdge(baseline.rows.sell);
  const bookBps = Math.max(Math.round(proof.sellerBps), Math.round(proof.buyerBps));
  const day = fmtDay(baseline.measuredAt);
  const sb = Math.round(proof.sellerBps);
  const bb = Math.round(proof.buyerBps);

  return (
    <section className="sec" id="route" aria-labelledby="route-h">
      <div className="bb-wrap">
        <RouteSideProvider>
          <header className="sec-head">
            <h2 className="h2" id="route-h">
              The route vs the&nbsp;book
            </h2>
            <div className="lead-col">
              <p className="lead">
                On {day}, selling {fmtCompact(largest.near)} NEAR through the public NEAR Intents route cost{" "}
                {Math.round(largest.costBps)}&nbsp;bps, about {fmtUsdK(largest.costUsd)}.
                {refused ? (
                  <>
                    {" "}
                    At {fmtCompact(refused.near)} the route answered “{refused.unavailable}”.
                  </>
                ) : null}{" "}
                Blindbook crosses at the mid, within {bookBps}&nbsp;bps, when the other side is in the batch.
              </p>
              <SideToggle />
            </div>
          </header>

          <RouteChart rows={rowViews(baseline, proof, bookBps)} />
        </RouteSideProvider>

        <div className="sr-only">
          <table>
            <caption>
              Public route cost against the midpoint, 1Click dry quotes, {fmtDayYear(baseline.measuredAt)}
            </caption>
            <thead>
              <tr>
                <th scope="col">Side</th>
                <th scope="col">Size (NEAR)</th>
                <th scope="col">Cost (bps)</th>
                <th scope="col">Cost (USD)</th>
              </tr>
            </thead>
            <tbody>
              {(["sell", "buy"] as const).flatMap((side) =>
                baseline.rows[side].map((r) => (
                  <tr key={`${side}-${r.near}`}>
                    <td>{side}</td>
                    <td>{fmtInt(r.near)}</td>
                    <td>{r.costBps == null ? r.unavailable : r.costBps.toFixed(2)}</td>
                    <td>{r.costUsd == null ? "" : r.costUsd.toFixed(2)}</td>
                  </tr>
                )),
              )}
              <tr>
                <td>either, on Blindbook</td>
                <td>any, when the other side is in the batch</td>
                <td>at most {bookBps} from the mid</td>
                <td />
              </tr>
            </tbody>
          </table>
        </div>

        <p className="cite">
          Measured {day}, 1Click dry quotes ({fmtTimeUtc(baseline.measuredAt)}, {baseline.pair}
          {baseline.exactInput ? ", exact input" : ""}, against a mid of {baseline.mid}). Source:{" "}
          <code>blindbook/results/{BASELINE_FILE}</code>. Blindbook&apos;s row is the {NETWORK.id} fill in batch {proof.id}: mid
          − {sb}&nbsp;bps for the seller, mid + {bb}&nbsp;bps for the buyer.
        </p>
      </div>
    </section>
  );
}
