import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test } from 'vitest';
import { slug } from '../src/writeup/render';

const docs = resolve(__dirname, '../../docs');
const md = readFileSync(resolve(docs, 'WRITEUP.md'), 'utf8');
const prose = md.replace(/```[\s\S]*?```/g, '');
const headings = [...md.matchAll(/^#{1,6} (.+)$/gm)].map((m) => m[1]);

test('every_image_exists', () => {
  const imgs = [...md.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)].map((m) => m[1]);
  expect(imgs.length).toBeGreaterThan(3);
  for (const src of imgs) {
    expect(src.startsWith('writeup-figures/'), src).toBe(true);
    expect(existsSync(resolve(docs, src)), src).toBe(true);
  }
});

test('every_anchor_resolves', () => {
  const ids = new Set(headings.map(slug));
  for (const m of md.matchAll(/\]\(#([^)]+)\)/g)) expect(ids.has(m[1]), m[1]).toBe(true);
});

test('length_is_in_range', () => {
  const words = prose.split(/\s+/).filter((w) => /[A-Za-z0-9]/.test(w)).length;
  expect(words).toBeGreaterThanOrEqual(2500);
  expect(words).toBeLessThanOrEqual(3500);
});

test('has_the_sections', () => {
  expect([...md.matchAll(/^## (.+)$/gm)].map((m) => m[1])).toEqual([
    'Why an airport',
    'Getting the data',
    'The twin: from positions to an airport',
    'A replay in the browser',
    'Forecasting with honest baselines',
    'Looking for unusual approaches',
    'What I would do differently',
    'Engineering notes',
  ]);
});
