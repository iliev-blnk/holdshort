import { Marked, Renderer, type Tokens } from 'marked';

const ENTITY: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ENTITY[c]);

/** GitHub-style heading id: lowercase, drop punctuation, spaces → hyphens. */
export function slug(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9 -]/g, '')
    .replace(/ /g, '-');
}

/** The write-up's Markdown as HTML: heading ids, external links in a new tab, scrollable
 * tables, lazy images after the first, and any raw HTML escaped. */
export function renderWriteup(md: string): string {
  const seen = new Map<string, number>();
  let images = 0;
  const base = new Renderer();
  const marked = new Marked({
    gfm: true,
    renderer: {
      heading({ tokens, depth, text }: Tokens.Heading) {
        let id = slug(text);
        const n = seen.get(id) ?? 0;
        seen.set(id, n + 1);
        if (n > 0) id = `${id}-${n}`;
        return `<h${depth} id="${id}">${this.parser.parseInline(tokens)}</h${depth}>\n`;
      },
      link({ href, tokens }: Tokens.Link) {
        const ext = /^https?:\/\//.test(href) ? ' target="_blank" rel="noopener"' : '';
        return `<a href="${esc(href)}"${ext}>${this.parser.parseInline(tokens)}</a>`;
      },
      image({ href, text }: Tokens.Image) {
        const lazy = images++ > 0 ? ' loading="lazy"' : '';
        return `<img src="${esc(href)}" alt="${esc(text)}"${lazy}>`;
      },
      table(token: Tokens.Table) {
        base.parser = this.parser;
        return `<div class="table">${base.table(token).trim()}</div>\n`;
      },
      html({ text }: Tokens.HTML | Tokens.Tag) {
        return esc(text);
      },
    },
  });
  return marked.parse(md, { async: false });
}
