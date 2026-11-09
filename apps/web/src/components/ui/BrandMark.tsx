/** The sealed-lot glyph: an open slot over a sealed one. Inherits `currentColor`. */
export function BrandMark({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={(size * 16) / 18} viewBox="0 0 18 16" aria-hidden="true" focusable="false">
      <rect x=".5" y=".5" width="17" height="6" rx="2" fill="none" stroke="currentColor" />
      <rect x="0" y="9" width="18" height="7" rx="2" fill="currentColor" />
    </svg>
  );
}

/** The check drawn on a filled lot, sized to one 36 × 22 tile. */
export function TileCheck() {
  return (
    <svg viewBox="0 0 36 22" aria-hidden="true" focusable="false">
      <path d="M13 11.5l3.5 3.5 7-7.5" />
    </svg>
  );
}
