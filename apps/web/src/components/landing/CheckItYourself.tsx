import Link from "next/link";
import { type CSSProperties, Suspense } from "react";
import { TileCheck } from "@/components/ui/BrandMark";
import { fmtDay, fmtInt, fmtPrice, shortHash } from "@/lib/content/format";
import { getPoolStats } from "@/lib/content/pool-stats";
import { accountUrl, attestationUrl, FUZZ, LINKS, NETWORK, TESTS, txUrl } from "@/lib/content/proof";
import type { BatchProof } from "@/lib/content/results";
import { CopyButton } from "./CopyButton";

const WORDS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
const word = (n: number) => WORDS[n] ?? String(n);
const cap = (t: string) => t[0].toUpperCase() + t.slice(1);
const lotsOf = (n: number, near: number) => `${word(n)} ${near} NEAR lot${n === 1 ? "" : "s"}`;

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
  const d = proof.priceDigits;
  return (
    <figure
      className="rail"
      style={railVars(proof)}
      aria-label={`Batch ${proof.id} on ${proof.net.label}: fills against both limits`}
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
          <b>{fmtPrice(proof.sellLimit, d)}</b>
          <span>seller&apos;s limit</span>
        </div>
        <div className="rl up at-mid">
          <b>{fmtPrice(proof.mid, 5)}</b>
          <span>1Click mid</span>
        </div>
        <div className="rl up at-bl">
          <b>{fmtPrice(proof.buyLimit, d)}</b>
          <span>buyer&apos;s limit</span>
        </div>
        <div className="rl dn at-sf">
          <b>{fmtPrice(proof.sellerPrice, d)}</b>
          <span>seller got, mid − {sb}&nbsp;bps</span>
        </div>
        <div className="rl dn at-bf">
          <b>{fmtPrice(proof.buyerPrice, d)}</b>
          <span>buyer paid, mid + {bb}&nbsp;bps</span>
        </div>
      </div>
      <figcaption>
        {proof.withinLimits ? "Both fills landed inside both limits. " : null}The lime spans are the price improvement,
        returned to each side in the same <code>execute_intents</code> call. Prices in USDC per NEAR,{" "}
        {lotsOf(proof.lots, proof.lotNear)} a side
        {proof.venueFee != null ? `. The venue kept ${proof.venueFee} USDC` : null}.
      </figcaption>
    </figure>
  );
}

/** The command that re-checks a batch from public sources, ready to copy. */
function VerifyCommand({ proof, title }: { proof: BatchProof; title: string }) {
  return (
    <div className="code-card verify-card">
      <div className="code-top">
        <span className="code-file">{title}</span>
        <CopyButton text={proof.verify} label="Copy" />
      </div>
      <pre>
        <code>{proof.verify}</code>
      </pre>
    </div>
  );
}

/** Batch count from the testnet pool itself; the e2e run stands in when the RPC is unreachable. */
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

/**
 * The headline batch (mainnet) with every artefact behind it, then a second
 * batch (testnet) in brief. Nothing here reads mainnet live: the numbers come
 * from the run records in `results/`, and the live counter reads the testnet
 * pool the demo trades on.
 */
export function CheckItYourself({ proof, second }: { proof: BatchProof; second: BatchProof }) {
  const net = proof.net;
  const crank = txUrl(proof.txs.crank, net);
  return (
    <section className="sec" id="check" aria-labelledby="check-h">
      <div className="bb-wrap">
        <header className="sec-head">
          <h2 className="h2" id="check-h">
            Check it yourself
          </h2>
          <p className="lead">
            Batch {proof.id} settled on {net.label} on {fmtDay(proof.at)}. <code>{proof.accounts.seller}</code> sold{" "}
            {lotsOf(proof.lots, proof.lotNear)} to <code>{proof.accounts.buyer}</code>, and both legs settled in one call
            on <code>{net.verifier}</code>. Every step left something you can open.
          </p>
        </header>

        <PriceRail proof={proof} />

        <div className="verify">
          <VerifyCommand proof={proof} title={`Re-check batch ${proof.id} on ${net.id}`} />
          <p className="verify-note">
            Seven checks, and every one should read <code>PASS</code>: the result signature, both legs in{" "}
            <code>execute_intents</code>, the signed 1Click quotes, the TDX attestation, the pinned build, the exact
            input and the venue key. It reads only public endpoints.
          </p>
        </div>

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
              <code title={net.matcher.wasmSha256}>wasm {shortHash(net.matcher.wasmSha256, 8, 4)}</code>
            </p>
            <a className="ck-link" href={attestationUrl(proof.outlayerRequest, net)} {...ext}>
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
              The sell, the buy and the venue&apos;s closure settled together on <code>{net.verifier}</code>. One call,
              all or nothing, improvement paid back inside it.
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

        <section className="also" aria-labelledby="also-h">
          <div className="also-copy">
            <h3 className="h3" id="also-h">
              Also on {second.net.label}: batch {second.id}
            </h3>
            <p>
              {cap(word(second.lots))} lot{second.lots === 1 ? "" : "s"} a side of {second.lotNear}&nbsp;NEAR each crossed
              at mid {fmtPrice(second.mid, 5)} on {fmtDay(second.at)}. The seller got {fmtPrice(second.sellerPrice, second.priceDigits)} against a limit of{" "}
              {fmtPrice(second.sellLimit, second.priceDigits)}. The buyer paid{" "}
              {fmtPrice(second.buyerPrice, second.priceDigits)} against {fmtPrice(second.buyLimit, second.priceDigits)}.
            </p>
            <p>
              The free demo runs on testnet. The same code runs on mainnet, batch {proof.id}.
            </p>
            <p className="ck-data">
              <a href={txUrl(second.txs.crank, second.net)} {...ext}>
                <code title={second.txs.crank}>tx {shortHash(second.txs.crank)}</code>
              </a>
              <a href={attestationUrl(second.outlayerRequest, second.net)} {...ext}>
                <code>attestation {second.outlayerRequest}</code>
              </a>
            </p>
          </div>
          <VerifyCommand proof={second} title={`Re-check batch ${second.id} on ${second.net.id}`} />
        </section>

        <dl className="facts">
          <div>
            <dt>Pool contracts</dt>
            <dd>
              <a href={accountUrl(proof.pool, net)} {...ext}>
                <code>{proof.pool}</code>
              </a>
              <span className="live">
                batch {proof.id} crossed {word(proof.lots)} lot{proof.lots === 1 ? "" : "s"} a side
              </span>
            </dd>
            <dd>
              <a href={accountUrl(NETWORK.pool, NETWORK)} {...ext}>
                <code>{NETWORK.pool}</code>
              </a>
              <Suspense fallback={<StaticPoolStats proof={second} />}>
                <LivePoolStats proof={second} />
              </Suspense>
            </dd>
          </div>
          <div>
            <dt>Matcher wasm, sha256</dt>
            <dd>
              <a href={net.matcher.wasmUrl} {...ext}>
                <code className="hash">{net.matcher.wasmSha256}</code>
              </a>
              <span className="live">
                {net.id} build ·{" "}
                <a href={second.net.matcher.wasmUrl} {...ext}>
                  {second.net.id} {shortHash(second.net.matcher.wasmSha256, 8, 4)}
                </a>
              </span>
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
