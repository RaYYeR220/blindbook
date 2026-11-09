import type { Metadata, Viewport } from "next";
import { Geist_Mono } from "next/font/google";
import "./globals.css";

/** General Sans is served by Fontshare's CDN, as its licence asks; the files are not redistributed here. */
const GENERAL_SANS = "https://api.fontshare.com/v2/css?f[]=general-sans@400,500,600&display=swap";

/** Numerals, hashes and code. */
const geistMono = Geist_Mono({
  subsets: ["latin"],
  variable: "--font-geist-mono",
  display: "swap",
});

const site =
  process.env.NEXT_PUBLIC_SITE_URL ??
  (process.env.VERCEL_PROJECT_PRODUCTION_URL
    ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
    : "http://localhost:3000");

const title = "Blindbook: sell size without being seen";
const description =
  "A dark pool for NEAR Intents. Seal a block at your own limit; an enclave crosses it at the signed 1Click midpoint, and both legs settle in one call.";

export const metadata: Metadata = {
  metadataBase: new URL(site),
  title: { default: title, template: "%s · Blindbook" },
  description,
  applicationName: "Blindbook",
  openGraph: { type: "website", siteName: "Blindbook", title, description },
  twitter: { card: "summary_large_image", title, description },
};

export const viewport: Viewport = {
  themeColor: "#2430d0",
  colorScheme: "dark",
};

/*
  Runs before first paint. `js` lets the hero hide its panel for the entrance;
  `is-in` is the failsafe that reveals it if the wall never boots.
*/
const BOOT = `(function(){var d=document.documentElement;d.classList.add("js");setTimeout(function(){d.classList.add("is-in")},1200)})()`;

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en" className={geistMono.variable} suppressHydrationWarning>
      <head>
        <link rel="preconnect" href="https://cdn.fontshare.com" crossOrigin="" />
        <link rel="stylesheet" href={GENERAL_SANS} />
        <script dangerouslySetInnerHTML={{ __html: BOOT }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
