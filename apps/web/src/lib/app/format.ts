import { type Market, priceOfLot } from "@blindbook/sdk";

/** Number formatting for the app. Prices, sizes and hashes render in the mono face. */

const en = (n: number, min: number, max = min) =>
  n.toLocaleString("en-US", { minimumFractionDigits: min, maximumFractionDigits: max });

/** A price in quote per base, e.g. 3.6413. */
export const fmtPrice = (p: number, digits = 4) => (Number.isFinite(p) ? p.toFixed(digits) : "–");

/** A token amount in smallest units, shown to `digits` decimals with grouping. */
export function fmtUnits(amount: bigint | string, decimals: number, digits = 2): string {
  const v = typeof amount === "string" ? BigInt(amount || "0") : amount;
  const neg = v < 0n;
  const abs = neg ? -v : v;
  const scale = 10n ** BigInt(decimals);
  const whole = abs / scale;
  const frac = ((abs % scale) * 10n ** BigInt(digits)) / scale;
  const s = `${whole.toLocaleString("en-US")}${digits > 0 ? `.${frac.toString().padStart(digits, "0")}` : ""}`;
  return neg ? `−${s}` : s;
}

/** Dollars: four decimals under $1 so a one-lot difference is still visible, cents above. */
export function fmtUsd(v: number): string {
  if (!Number.isFinite(v)) return "–";
  const a = Math.abs(v);
  const s = a < 1 ? `$${a.toFixed(4)}` : a < 10_000 ? `$${en(a, 2)}` : `$${en(Math.round(a), 0)}`;
  return v < 0 ? `−${s}` : s;
}

/** Basis points to one decimal: 30.6 bps. */
export const fmtBps = (b: number) => `${en(b, 1)} bps`;

/** Signed basis points, e.g. "−5.0 bps", "+0.6 bps". */
export const fmtSignedBps = (b: number) => `${b < 0 ? "−" : "+"}${en(Math.abs(b), 1)} bps`;

/** Shorten a hash or key: first and last characters around an ellipsis. */
export const short = (h: string, head = 6, tail = 4) => (h.length <= head + tail + 1 ? h : `${h.slice(0, head)}…${h.slice(-tail)}`);

/** Shorten an account id only when it is long, as implicit and 0x accounts are. */
export const shortAccount = (a: string) => (a.length > 24 ? short(a, 8, 6) : a);

/** A lot's quote amount as a price per whole base unit. */
export const lotPrice = (market: Market, quote: bigint | string) =>
  Number(priceOfLot(market, typeof quote === "string" ? BigInt(quote) : quote, 8));

/** A NEAR block timestamp (ns since epoch, as a string) to a Date. */
export const nsToDate = (ns: string) => new Date(Number(BigInt(ns) / 1_000_000n));

/** "14:33:07" in the viewer's own clock. */
export const fmtClock = (d: Date) => d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" });

/** How long ago, coarse: "12 s ago", "4 min ago", "3 h ago". */
export function fmtAgo(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

/** Remaining time, coarse: "9 min", "5 h 40 min". */
export function fmtLeft(ms: number): string {
  if (ms <= 0) return "expired";
  const m = Math.ceil(ms / 60_000);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}

/** Bytes with grouping: "4,169 B". */
export const fmtBytes = (n: number) => `${n.toLocaleString("en-US")} B`;

/** Basis points between a fill and the mid, signed from the trader's side: negative is a cost. */
export function bpsFromMid(side: "sell" | "buy", fill: number, mid: number): number {
  return side === "sell" ? ((fill - mid) / mid) * 10_000 : ((mid - fill) / mid) * 10_000;
}
