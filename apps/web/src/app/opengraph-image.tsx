import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { ImageResponse } from "next/og";
import { hexFrom } from "@/components/tiles/draw";
import { GROUND, LIME, PX, PY, TH, TILE, TW } from "@/components/tiles/palette";

export const alt = "Blindbook: a wall of identical sealed orders on ultramarine, with the line “Sell size without being seen.”";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

const fonts = join(process.cwd(), "src", "fonts");
/** General Sans Semibold comes from Fontshare's CDN at build time; the font files are not redistributed. */
async function generalSansSemibold(): Promise<ArrayBuffer | null> {
  try {
    const css = await fetch("https://api.fontshare.com/v2/css?f[]=general-sans@600&display=swap").then((r) => r.text());
    const woff = /url\('([^']+\.woff)'\) format\('woff'\)/.exec(css)?.[1];
    if (!woff) return null;
    return await fetch(woff.startsWith("//") ? `https:${woff}` : woff).then((r) => r.arrayBuffer());
  } catch {
    return null;
  }
}
const mono = await readFile(join(fonts, "GeistMono-Regular.ttf"));

/** A few lots mid-cross, as on the hero wall: [column, row, fill]. */
const LIT: [number, number, string][] = [
  [3, 3, LIME],
  [2, 13, LIME],
  [20, 2, LIME],
  [23, 10, "#ffffff"],
  [14, 16, LIME],
];
const EMPTY = new Set(["9,1", "14,7", "2,9", "22,5", "6,0", "16,10"]);

export default async function Image() {
  const sansSemibold = await generalSansSemibold();
  const cols = Math.ceil(size.width / PX) + 1;
  const rows = Math.ceil(size.height / PY) + 1;
  const x0 = (size.width - cols * PX) / 2 + (PX - TW) / 2;
  const tiles = [];
  for (let k = 0; k < rows; k++) {
    for (let j = 0; j < cols; j++) {
      const lit = LIT.find(([c, r]) => c === j && r === k);
      const empty = EMPTY.has(`${j},${k}`);
      tiles.push(
        <div
          key={`${j},${k}`}
          style={{
            position: "absolute",
            left: x0 + j * PX,
            top: 6 + k * PY,
            width: TW,
            height: TH,
            borderRadius: 3,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: lit ? lit[2] : empty ? "transparent" : TILE,
            border: empty ? "1px solid rgba(255,255,255,.18)" : lit ? "none" : "1px solid rgba(255,255,255,.10)",
            fontFamily: "Geist Mono",
            fontSize: 8.5,
            color: "rgba(255,255,255,.25)",
          }}
        >
          {lit ? (
            <svg width="36" height="22" viewBox="0 0 36 22">
              <path d="M13 11.5l3.5 3.5 7-7.5" fill="none" stroke={GROUND} strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          ) : empty ? null : (
            hexFrom(j * 41 + k * 7 + 3)
          )}
        </div>,
      );
    }
  }

  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", position: "relative", background: GROUND }}>
        {tiles}
        <div
          style={{
            position: "absolute",
            left: 170,
            top: 132,
            width: 860,
            height: 366,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            borderRadius: 40,
            background: "rgba(36,48,208,.92)",
            border: "1px solid rgba(255,255,255,.12)",
            color: "#ffffff",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 12, fontFamily: "General Sans", fontSize: 26, letterSpacing: "-0.02em" }}>
            <svg width="27" height="24" viewBox="0 0 18 16">
              <rect x=".5" y=".5" width="17" height="6" rx="2" fill="none" stroke="#ffffff" />
              <rect x="0" y="9" width="18" height="7" rx="2" fill="#ffffff" />
            </svg>
            Blindbook
          </div>
          <div
            style={{
              marginTop: 26,
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              fontFamily: "General Sans",
              fontSize: 92,
              lineHeight: 0.96,
              letterSpacing: "-0.05em",
            }}
          >
            <span>Sell size without</span>
            <span>being seen.</span>
          </div>
          <div style={{ marginTop: 30, fontFamily: "Geist Mono", fontSize: 18, color: "rgba(255,255,255,.66)" }}>
            a dark pool for NEAR Intents
          </div>
        </div>
      </div>
    ),
    {
      ...size,
      fonts: [
        ...(sansSemibold ? [{ name: "General Sans", data: sansSemibold, style: "normal" as const, weight: 600 as const }] : []),
        { name: "Geist Mono", data: mono, style: "normal", weight: 400 },
      ],
    },
  );
}
