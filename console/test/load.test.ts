import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { afterEach, expect, test, vi } from 'vitest';
import { latestOnly, loadDay } from '../src/data/load';

const dir = new URL('./fixture/', import.meta.url);

afterEach(() => vi.unstubAllGlobals());

test('loadDay fetches day.json and the gzipped tracks under base/airport/date', async () => {
  const urls: string[] = [];
  vi.stubGlobal('fetch', async (url: string) => {
    urls.push(url);
    const b = readFileSync(new URL(url.replace(/^data\//, ''), dir));
    return new Response(b);
  });
  const { day, tracks } = await loadDay('data', 'LTFM', '2026-10-02');
  expect(urls.sort()).toEqual(['data/LTFM/2026-10-02/day.json', 'data/LTFM/2026-10-02/tracks.bin.gz']);
  expect(day.flights.length).toBe(3);
  expect(tracks.n).toBe(32);
});

test('loadDay accepts tracks a server already unpacked (Content-Encoding: gzip)', async () => {
  vi.stubGlobal('fetch', async (url: string) => {
    const b = readFileSync(new URL(url.replace(/^data\//, ''), dir));
    return new Response(url.endsWith('.gz') ? gunzipSync(b) : b);
  });
  const { tracks } = await loadDay('data', 'LTFM', '2026-10-02');
  expect(tracks.n).toBe(32);
});

test('loadDay rejects on a missing file', async () => {
  vi.stubGlobal('fetch', async () => new Response('nope', { status: 404 }));
  await expect(loadDay('data', 'LTFM', '2026-10-09')).rejects.toThrow();
});

test('latestOnly drops a superseded result', async () => {
  const wait = (ms: number, v: string) => new Promise<string>((r) => setTimeout(() => r(v), ms));
  const load = latestOnly((ms: number, v: string) => wait(ms, v));
  const a = load(30, 'A');
  const b = load(5, 'B');
  expect(await b).toBe('B');
  expect(await a).toBeUndefined();
});
