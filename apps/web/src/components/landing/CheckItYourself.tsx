import Link from "next/link";
import { type CSSProperties, Suspense } from "react";
import { TileCheck } from "@/components/ui/BrandMark";
import { fmtDay, fmtInt, fmtPrice, shortHash } from "@/lib/content/format";
import { getPoolStats } from "@/lib/content/pool-stats";
import { accountUrl, attestationUrl, FUZZ, LINKS, MATCHER, NETWORK, TESTS, txUrl } from "@/lib/content/proof";
import type { BatchProof } from "@/lib/content/results";

const WORDS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
const word = (n: number) => WORDS[n] ?? String(n);
const cap = (t: string) => t[0].toUpperCase() + t.slice(1);

/** Both limits at the rail's ends (6% and 94%), the mid and both fills between them, to scale. */
function railVars(p: BatchProof): CSSProperties {
  const at = (price: number) => `${(6 + ((price - p.sellLimit) / (p.buyLimit - p.sellLimit)) * 88).toFixed(2)}%`;
  return {
    "--sl": at(p.sellLimit),
    "--sf": at(p.sellerPrice),
    "--mid": at(p.mid),
    "--bf": at(p.buyerPrice),
    "--bl": at(p.buyLimit),
  } as CSSProperties;
}

function PriceRail({ proof }: { proof: BatchProof }) {
  const sb = Math.round(proof.sellerBps);
  const bb = Math.round(proof.buyerBps);
  return (
    <figure
      className="rail"
      style={railVars(proof)}
      aria-label={`Batch ${proof.id} fills against both limits`}
    >
      <div className="rail-track">
        <div className="rail-art" aria-hidden="true">
          <div className="rail-line" />
          <div className="rail-span s" />
          <div className="rail-span b" />
          <div className="rail-band" />
          <div className="rail-tick at-sl" />
          <div className="rail-tick at-mid" />
          <div className="rail-tick at-bl" />
          <div className="rail-flash" />
          <div className="rail-tile s">
            <TileCheck />
          </div>
          <div className="rail-tile b">
            <TileCheck />
          </div>
        </div>
        <div className="rl up at-sl">
          <b>{fmtPrice(proof.sellLimit, 4)}</b>
          <span>seller&apos;s limit</span>
        </div>
        <div className="rl up at-mid">
          <b>{fmtPrice(proof.mid, 5)}</b>
          <span>1Click mid</span>
        </div>
        <div className="rl up at-bl">
          <b>{fmtPrice(proof.buyLimit, 4)}</b>
          <span>buyer&apos;s limit</span>
        </div>
        <div className="rl dn at-sf">
          <b>{fmtPrice(proof.sellerPrice, 4)}</b>
          <span>seller got, mid − {sb}&nbsp;bps</span>
        </div>
        <div className="rl dn at-bf">
          <b>{fmtPrice(proof.buyerPrice, 4)}</b>
          <span>buyer paid, mid + {bb}&nbsp;bps</span>
        </div>
      </div>
      <figcaption>
        {proof.withinLimits ? "Both fills landed inside both limits. " : null}The lime spans are the price improvement,
        returned to each side in the same <code>execute_intents</code> call. Prices in USDC per NEAR, {word(proof.lots)}{" "}
        {proof.lotNear}&nbsp;NEAR lots a side.
      </figcaption>
    </figure>
  );
}

/** Batch count from the pool itself; the e2e run stands in when the RPC is unreachable. */
function StaticPoolStats({ proof }: { proof: BatchProof }) {
  return (
    <span className="live">
      batch {proof.id} crossed {word(proof.lots)} lots a side
    </span>
  );
}

async function LivePoolStats({ proof }: { proof: BatchProof }) {
  const stats = await getPoolStats();
  if (!stats) return <StaticPoolStats proof={proof} />;
  return (
    <span className="live">
      <i aria-hidden="true" />
      {fmtInt(stats.batches)} batches run, {fmtInt(stats.crossedLots)} lots crossed
    </span>
  );
}

const ext = { target: "_blank", rel: "noopener" } as const;

export function CheckItYourself({ proof }: { proof: BatchProof }) {
  const crank = txUrl(proof.txs.crank);
  return (
    <section className="sec" id="check" aria-labelledby="check-h">
      <div className="bb-wrap">
        <header className="sec-head">
          <h2 className="h2" id="check-h">
            Check it yourself
          </h2>
          <p className="lead">
            Batch {proof.id} ran end to end on {NETWORK.label} on {fmtDay(proof.at)}. Two traders sealed opposite orders,
            a third account cranked the batch, and every step left something you can open.
          </p>
        </header>

        <PriceRail proof={proof} />

        <ul className="checks">
          <li className="ck">
            <span className="ck-tile" aria-hidden="true">
              <TileCheck />
            </span>
            <h3 className="h3">1Click quote signatures</h3>
            <p>
              The enclave fetched a bid and an ask quote, verified 1Click&apos;s ed25519 signatures, and wrote both
              digests into its signed result.
            </p>
            <p className="ck-data">
              {proof.quoteDigests.map((d) => (
                <code key={d} title={d}>
                  {shortHash(d)}
                </code>
              ))}
            </p>
            <a className="ck-link" href={crank} {...ext}>
              See them in the tx
            </a>
          </li>
          <li className="ck">
            <span className="ck-tile" aria-hidden="true">
              <TileCheck />
            </span>
            <h3 className="h3">TDX attestation</h3>
            <p>
              OutLayer publishes the Intel TDX quote for every run, bound to the matcher&apos;s wasm hash and to the
              batch&apos;s input and output.
            </p>
            <p className="ck-data">
              <code>request {proof.outlayerRequest}</code>
              <code title={MATCHER.wasmSha256}>wasm {shortHash(MATCHER.wasmSha256, 8, 4)}</code>
            </p>
            <a className="ck-link" href={attestationUrl(proof.outlayerRequest)} {...ext}>
              Open the attestation
            </a>
          </li>
          <li className="ck">
            <span className="ck-tile" aria-hidden="true">
              <TileCheck />
            </span>
            <h3 className="h3">Result signature, checked on chain</h3>
            <p>
              Before acting, the pool verified the matcher&apos;s signature against its pinned key with{" "}
              <code>env::ed25519_verify</code>.
            </p>
            <p className="ck-data">
              <code title={proof.resultHash}>result {shortHash(proof.resultHash, 8, 4)}</code>
            </p>
            <a className="ck-link" href={crank} {...ext}>
              See <code>on_result</code>
            </a>
          </li>
          <li className="ck">
            <span className="ck-tile" aria-hidden="true">
              <TileCheck />
            </span>
            <h3 className="h3">
              <code>execute_intents</code> on the verifier
            </h3>
            <p>
              {cap(word(proof.lots))} sells, {word(proof.lots)} buys and the venue&apos;s closure settled together. One
              call, all or nothing, improvement paid back inside it.
            </p>
            <p className="ck-data">
              <code title={proof.txs.crank}>{shortHash(proof.txs.crank)}</code>
            </p>
            <a className="ck-link" href={crank} {...ext}>
              Open on nearblocks
            </a>
          </li>
          <li className="ck is-spec">
            <span className="ck-tile" aria-hidden="true">
              <TileCheck />
            </span>
            <h3 className="h3">Self-cancel ticket</h3>
            <p>
              Sign the same nonce with no intents and send it to the verifier yourself. The venue isn&apos;t asked.
              Nobody cancelled in batch {proof.id}.
            </p>
            <p className="ck-data">
              <code>&quot;intents&quot;: []</code>
            </p>
            <Link className="ck-link" href={LINKS.cancelTicket}>
              Read the ticket format
            </Link>
          </li>
        </ul>

        <dl className="facts">
          <div>
            <dt>Pool contract</dt>
            <dd>
              <a href={accountUrl(proof.pool)} {...ext}>
                <code>{proof.pool}</code>
              </a>
              <Suspense fallback={<StaticPoolStats proof={proof} />}>
                <LivePoolStats proof={proof} />
              </Suspense>
            </dd>
          </div>
          <div>
            <dt>Matcher wasm, sha256</dt>
            <dd>
              <a href={MATCHER.wasmUrl} {...ext}>
                <code className="hash">{MATCHER.wasmSha256}</code>
              </a>
            </dd>
          </div>
          <div>
            <dt>Tests</dt>
            <dd>
              core {TESTS.core}, matcher {TESTS.matcher}, pool {TESTS.pool}, SDK {TESTS.sdk}
            </dd>
          </div>
          <div>
            <dt>Engine fuzz</dt>
            <dd>
              {fmtInt(FUZZ.books)} random books, {FUZZ.violations} invariant violations
            </dd>
          </div>
        </dl>
      </div>
    </section>
  );
}
