import { fmtDay, fmtPrice } from "@/lib/content/format";
import { NETWORK } from "@/lib/content/proof";
import type { BatchProof } from "@/lib/content/results";
import { type Step, StepScenes } from "./StepScenes";

const STEPS: Step[] = [
  {
    title: "Seal",
    body: (
      <>
        Split the block into lots and sign each one at your limit. The order is encrypted to the enclave&apos;s key and
        padded to a fixed size, so a 10&nbsp;NEAR order and a 50k one look the same.
      </>
    ),
  },
  {
    title: "Batch",
    body: (
      <>
        Anyone can crank <code>run_batch()</code>. The pool hands every sealed order to the matcher inside an Intel TDX
        enclave, in one request.
      </>
    ),
  },
  {
    title: "Cross at the mid",
    body: (
      <>
        The enclave checks two 1Click quotes signed with ed25519 and takes their midpoint. Every compatible lot crosses at
        that one price, for both sides alike.
      </>
    ),
  },
  {
    title: "Settle",
    body: (
      <>
        Both legs settle in one <code>execute_intents</code> call. The gap between each limit and the mid, less the venue
        fee, goes back to both sides in that same call.
      </>
    ),
  },
];

export function HowItCrosses({ proof }: { proof: BatchProof }) {
  return (
    <section className="sec" id="how" aria-labelledby="how-h">
      <div className="bb-wrap">
        <header className="sec-head">
          <h2 className="h2" id="how-h">
            How a block crosses
          </h2>
          <p className="lead">
            Four steps, once per batch. No server ever holds your order, your keys or your funds: the pool contract stores
            ciphertext, the enclave matches, and the NEAR Intents verifier settles.
          </p>
        </header>

        <StepScenes
          steps={STEPS}
          labels={{ sellLimit: fmtPrice(proof.sellLimit, 4), mid: `mid ${fmtPrice(proof.mid, 5)}` }}
        />
        <p className="fine">
          Prices in the scenes are from batch {proof.id} on {NETWORK.label}, {fmtDay(proof.at)}. Tile positions are
          illustrative.
        </p>

        <div className="worst">
          <p className="worst-line">Worst case: your own&nbsp;limit.</p>
          <p className="worst-sub">
            Each lot is signed at its limit, and the verifier applies signed amounts exactly. Whatever the matcher does,
            no lot can settle at a worse price than the one you signed.
          </p>
        </div>
      </div>
    </section>
  );
}
