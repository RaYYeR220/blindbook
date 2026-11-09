import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Marked } from "marked";

export interface ProtocolDoc {
  title: string;
  html: string;
  toc: { id: string; text: string }[];
}

/** "1.4 Cancelling" → "14-cancelling" */
export const slug = (text: string) =>
  text
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");

const escapeHtml = (t: string) =>
  t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** The repo's protocol spec. The app builds and runs from apps/web. */
const PROTOCOL_MD = join(process.cwd(), "..", "..", "docs", "PROTOCOL.md");

/** docs/PROTOCOL.md as HTML, with anchor ids on every heading. */
export function loadProtocol(): ProtocolDoc {
  const src = readFileSync(PROTOCOL_MD, "utf8");
  const toc: ProtocolDoc["toc"] = [];
  let title = "Protocol";

  const md = new Marked({ gfm: true });
  md.use({
    renderer: {
      heading({ tokens, depth, text }) {
        const inner = this.parser.parseInline(tokens);
        if (depth === 1) {
          title = text;
          return "";
        }
        const id = slug(text);
        if (depth === 2) toc.push({ id, text });
        return `<h${depth} id="${id}"><a href="#${id}">${inner}</a></h${depth}>\n`;
      },
      // the spec is plain markdown: raw HTML in it is shown as text, never injected
      html({ text }) {
        return escapeHtml(text);
      },
      table(token) {
        const head = token.header.map((c) => this.tablecell(c)).join("");
        const body = token.rows.map((row) => `<tr>${row.map((c) => this.tablecell(c)).join("")}</tr>`).join("\n");
        return `<div class="table-scroll"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>\n`;
      },
    },
  });

  const html = md.parse(src, { async: false });
  return { title, html, toc };
}
