// Yoru's answers as Markdown: the model writes **bold**, lists, tables and
// ```mermaid diagrams, and the chat shows them as such. Answers can carry
// text from web pages and connectors, so raw HTML is shown as text, images
// become links (nothing is fetched), and the result goes through DOMPurify.
// Mermaid is heavy: it's loaded the first time a diagram is shown.

import DOMPurify from "dompurify";
import { Marked, type Tokens } from "marked";

const escape = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

/** Only web and mail addresses become links; anything else stays text. */
const safeHref = (href: string) => /^(https?:|mailto:)/i.test(href.trim());

const marked = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    // Raw HTML in an answer is something to read, not to run.
    html: ({ text }: Tokens.HTML | Tokens.Tag) => escape(text),
    image: ({ href, text }: Tokens.Image) =>
      safeHref(href) ? `<a href="${escape(href)}">${escape(text || href)}</a>` : escape(text),
    link({ href, title, tokens }: Tokens.Link) {
      const inner = this.parser.parseInline(tokens);
      if (!safeHref(href)) return inner;
      return `<a href="${escape(href)}"${title ? ` title="${escape(title)}"` : ""}>${inner}</a>`;
    },
    code({ text, lang }: Tokens.Code) {
      const code = `<pre><code>${escape(text)}</code></pre>`;
      if ((lang ?? "").trim().toLowerCase() !== "mermaid") return code;
      // The source rides along, encoded (DOMPurify drops attributes holding "-->");
      // the component draws it, keeping the code if it can't.
      return `<div class="mermaid" data-src="${encodeURIComponent(text)}">${code}</div>`;
    },
  },
});

/** Mid-answer the last fence may still be open: close it, and keep an open mermaid block as code. */
function closeFences(text: string): string {
  const fences = [...text.matchAll(/^ {0,3}(```|~~~)/gm)];
  if (fences.length % 2 === 0) return text;
  const last = fences[fences.length - 1];
  const head = text.slice(0, last.index);
  const body = text.slice(last.index).replace(/^(\s*(```|~~~))\s*mermaid\b/i, "$1");
  return `${head}${body}\n${last[1]}`;
}

/** The answer as safe HTML. */
export function markdown(text: string): string {
  const html = marked.parse(closeFences(text), { async: false });
  return DOMPurify.sanitize(html, { ADD_ATTR: ["data-src"], FORBID_TAGS: ["img", "style", "form", "input"] });
}

type Mermaid = typeof import("mermaid").default;
let loading: Promise<Mermaid> | null = null;
let theme = "";
let ids = 0;
const drawn = new Map<string, string | null>();

async function mermaidFor(dark: boolean): Promise<Mermaid> {
  loading ??= import("mermaid").then((m) => m.default);
  const m = await loading;
  const want = dark ? "dark" : "neutral";
  if (theme !== want) {
    m.initialize({ startOnLoad: false, securityLevel: "strict", theme: want, fontFamily: "-apple-system, system-ui, sans-serif" });
    theme = want;
  }
  return m;
}

/** A diagram as SVG, or null when Mermaid can't read it; remembered per theme. */
export async function diagram(src: string, dark: boolean): Promise<string | null> {
  const key = `${dark ? "d" : "l"}:${src}`;
  if (drawn.has(key)) return drawn.get(key)!;
  const m = await mermaidFor(dark);
  const id = `yoru-mermaid-${++ids}`;
  let svg: string | null = null;
  try {
    if (await m.parse(src, { suppressErrors: true })) svg = (await m.render(id, src)).svg;
  } catch {
    svg = null;
  }
  // A failed render can leave its scratch element in the page.
  document.getElementById(id)?.remove();
  document.getElementById(`d${id}`)?.remove();
  drawn.set(key, svg);
  return svg;
}
