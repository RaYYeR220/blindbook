import type { Metadata } from "next";
import { SiteFooter } from "@/components/landing/SiteFooter";
import { SiteHeader } from "@/components/landing/SiteHeader";
import { loadProtocol } from "@/lib/content/protocol";
import "@/styles/protocol.css";

export const metadata: Metadata = {
  title: "Protocol",
  description: "How Blindbook seals, batches, crosses and settles orders on NEAR Intents: the full protocol spec.",
};

export default function ProtocolPage() {
  const doc = loadProtocol();
  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <SiteHeader current="protocol" />
      <main id="main" className="protocol-page">
        <aside className="toc" aria-label="Sections">
          <p className="toc-head">On this page</p>
          <ol>
            {doc.toc.map((t) => (
              <li key={t.id}>
                <a href={`#${t.id}`}>{t.text}</a>
              </li>
            ))}
          </ol>
        </aside>
        <article className="doc">
          <h1 className="title">{doc.title}</h1>
          <div className="body" dangerouslySetInnerHTML={{ __html: doc.html }} />
        </article>
      </main>
      <SiteFooter />
    </>
  );
}
