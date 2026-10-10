import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { decodeTracks } from '../src/data/decode';
import type { DayJson, Tracks } from '../src/data/types';

/** Tracks from [tMs, lat, lon, trk|null] rows (one flight, slice [0, n]). */
export function mk(points: [number, number, number, number | null][]): Tracks {
  const n = points.length;
  return {
    n,
    dayStartMs: 0,
    t: Int32Array.from(points, (p) => Math.round(p[0] / 100)),
    lat: Int32Array.from(points, (p) => Math.round(p[1] * 1e5)),
    lon: Int32Array.from(points, (p) => Math.round(p[2] * 1e5)),
    alt: Int16Array.from(points, () => 5000),
    trk: Uint16Array.from(points, (p) => (p[3] === null ? 65535 : Math.round(p[3] * 100))),
    gs: Uint16Array.from(points, () => 2500),
  };
}

const dir = new URL('./fixture/LTFM/2026-10-02/', import.meta.url);

export function fixture(): { day: DayJson; tracks: Tracks } {
  const b = gunzipSync(readFileSync(new URL('tracks.bin.gz', dir)));
  const buf = b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer;
  return { day: JSON.parse(readFileSync(new URL('day.json', dir), 'utf8')), tracks: decodeTracks(buf) };
}

export const hm = (h: number, m: number) => (h * 60 + m) * 60_000;
