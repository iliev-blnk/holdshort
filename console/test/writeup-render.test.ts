import { expect, test } from 'vitest';
import { renderWriteup, slug } from '../src/writeup/render';

test('heading_ids', () => {
  const html = renderWriteup('# A b\n\n## A b\n\n### Why: it’s 3.5!');
  expect(html).toContain('<h1 id="a-b">A b</h1>');
  expect(html).toContain('<h2 id="a-b-1">A b</h2>');
  expect(html).toContain('id="why-its-35"');
});

test('slug matches GitHub style', () => {
  expect(slug('The twin: from positions to an airport')).toBe('the-twin-from-positions-to-an-airport');
  expect(slug('What I would do differently')).toBe('what-i-would-do-differently');
});

test('external_links', () => {
  const html = renderWriteup('[x](https://example.com) and [y](#a-b)');
  expect(html).toContain('<a href="https://example.com" target="_blank" rel="noopener">x</a>');
  expect(html).toContain('<a href="#a-b">y</a>');
});

test('tables_are_wrapped', () => {
  const html = renderWriteup('| a | b |\n| --- | --- |\n| 1 | 2 |');
  expect(html).toContain('<div class="table"><table>');
  expect(html).toContain('</table></div>');
});

test('raw_html_is_escaped', () => {
  const html = renderWriteup('<script>alert(1)</script>\n\ntext <b>x</b>');
  expect(html).not.toContain('<script>');
  expect(html).not.toContain('<b>');
  expect(html).toContain('&lt;script&gt;');
});

test('images_keep_src_and_lazy_load_after_the_first', () => {
  const html = renderWriteup('![one](writeup-figures/day.png)\n\n![two](writeup-figures/b.png)');
  expect(html).toContain('<img src="writeup-figures/day.png" alt="one">');
  expect(html).toContain('<img src="writeup-figures/b.png" alt="two" loading="lazy">');
});
