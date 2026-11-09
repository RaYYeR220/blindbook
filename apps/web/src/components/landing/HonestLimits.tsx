const LIMITS = [
  {
    title: "Settlement is public.",
    body: "After a cross, both legs settle on intents.near, where anyone can read them. Before the cross nothing is public: resting orders, their sizes and their limits never leave the enclave.",
  },
  {
    title: "Early liquidity is our own market maker.",
    body: "Until outside flow arrives, a house market maker quotes the other side. It trades from a labelled house account, so you can see when you crossed with it.",
  },
  {
    title: "You trust TDX hardware and OutLayer's operator.",
    body: "A compromised enclave could leak what your order says. It still couldn't fill you worse than the limit you signed.",
  },
  {
    title: "Live on testnet. Mainnet is pending.",
    body: "Mainnet lots will be 0.1 NEAR. Until the first mainnet batch settles, everything on this page ran on NEAR testnet.",
  },
];

export function HonestLimits() {
  return (
    <section className="sec" id="limits" aria-labelledby="limits-h">
      <div className="bb-wrap limits">
        <header className="limits-head">
          <h2 className="h2" id="limits-h">
            Honest limits
          </h2>
          <p className="lead">What Blindbook doesn&apos;t hide, and who you still trust.</p>
        </header>
        <ul className="limit-list">
          {LIMITS.map((l) => (
            <li key={l.title}>
              <h3 className="h3">{l.title}</h3>
              <p>{l.body}</p>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
