const en = (n: number, digits = 0) =>
  n.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits });

/** 150000 → "150,000" */
export const fmtInt = (n: number) => en(n);

/** 150000 → "150k" */
export const fmtCompact = (n: number) => (n >= 1000 && n % 1000 === 0 ? `${n / 1000}k` : en(n));

/** Cents under $10, whole dollars above. */
export const fmtUsd = (v: number) => (v < 10 ? `$${v.toFixed(2)}` : `$${en(Math.round(v))}`);

/** 5409.18 → "$5.4k" */
export const fmtUsdK = (v: number) => (v >= 1000 ? `$${(v / 1000).toFixed(1)}k` : fmtUsd(v));

/** A price to a fixed number of decimals, without grouping. */
export const fmtPrice = (p: number, digits: number) => p.toFixed(digits);

/** "2026-11-09T14:33:41Z" → "Nov 9" */
export const fmtDay = (iso: string) =>
  new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });

/** "2026-11-09T14:33:41Z" → "Nov 9 2026" */
export const fmtDayYear = (iso: string) => `${fmtDay(iso)} ${new Date(iso).getUTCFullYear()}`;

/** "2026-11-09T14:33:41Z" → "14:33 UTC" */
export const fmtTimeUtc = (iso: string) => `${new Date(iso).toISOString().slice(11, 16)} UTC`;

/** Shorten a hash for a chip: first and last characters around an ellipsis. */
export const shortHash = (h: string, head = 6, tail = 4) => `${h.slice(0, head)}…${h.slice(-tail)}`;
