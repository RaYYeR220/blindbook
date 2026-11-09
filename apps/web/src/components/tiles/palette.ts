/**
 * The tile language shared by every canvas scene and the OG image. Colours
 * mirror the tokens in src/app/globals.css.
 */
export const GROUND = "#2430d0";
export const DEEP = "#1e28b6";
export const TILE = "#3843dc";
export const LIME = "#d4f24a";
export const WHITE = "#ffffff";

/** One sealed lot: every tile is the same size, like the padded ciphertext. */
export const TW = 36;
export const TH = 22;
/** Pitch of the tile grid. */
export const PX = 48;
export const PY = 34;
export const RAD = 3;

export interface Faces {
  sans: string;
  mono: string;
}

const FALLBACK: Faces = {
  sans: 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  mono: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
};

/** Font stacks as resolved on the page, so canvas text uses the self-hosted faces. */
export function readFaces(): Faces {
  const css = getComputedStyle(document.documentElement);
  return {
    sans: css.getPropertyValue("--font-sans").trim() || FALLBACK.sans,
    mono: css.getPropertyValue("--font-mono").trim() || FALLBACK.mono,
  };
}

export const prefersReducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
