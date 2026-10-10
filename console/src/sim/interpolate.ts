import { altAt, trkAt } from '../data/decode';
import type { Tracks } from '../data/types';

/** Longer gaps between points are coverage holes: the aircraft is hidden. */
export const GAP_MS = 300_000;

export interface Pos {
  lon: number;
  lat: number;
  alt: number | null;
  /** Degrees clockwise from north. */
  trk: number;
}

/** Index of the last point in [s, e) with t <= tDs, or s - 1. */
function floorIndex(t: Int32Array, s: number, e: number, tDs: number): number {
  let lo = s;
  let hi = e;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (t[mid] <= tDs) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

function bearing(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const r = Math.PI / 180;
  const y = Math.sin((lon2 - lon1) * r) * Math.cos(lat2 * r);
  const x = Math.cos(lat1 * r) * Math.sin(lat2 * r) - Math.sin(lat1 * r) * Math.cos(lat2 * r) * Math.cos((lon2 - lon1) * r);
  return ((Math.atan2(y, x) / r) % 360 + 360) % 360;
}

function lerpAngle(a: number, b: number, f: number): number {
  const d = ((b - a + 540) % 360) - 180;
  return (((a + d * f) % 360) + 360) % 360;
}

function at(tr: Tracks, i: number, j: number, f: number): Pos {
  const lat = (tr.lat[i] + (tr.lat[j] - tr.lat[i]) * f) / 1e5;
  const lon = (tr.lon[i] + (tr.lon[j] - tr.lon[i]) * f) / 1e5;
  const ai = altAt(tr, i);
  const aj = altAt(tr, j);
  const alt = ai !== null && aj !== null ? ai + (aj - ai) * f : (ai ?? aj);
  const ti = trkAt(tr, i);
  const tj = trkAt(tr, j);
  let trk: number;
  if (ti !== null && tj !== null) trk = lerpAngle(ti, tj, f);
  else if (j !== i) trk = bearing(tr.lat[i] / 1e5, tr.lon[i] / 1e5, tr.lat[j] / 1e5, tr.lon[j] / 1e5);
  else trk = ti ?? tj ?? 0;
  return { lon, lat, alt, trk };
}

/** Position of the flight whose points are `p` = [start, count] at `tMs` (ms since day start). */
export function positionAt(tr: Tracks, p: [number, number], tMs: number): Pos | null {
  const [s, c] = p;
  if (c === 0) return null;
  const e = s + c;
  const tDs = tMs / 100;
  const i = floorIndex(tr.t, s, e, tDs);
  if (i < s) return null;
  if (i === e - 1) {
    if (tr.t[i] !== tDs) return null;
    // A lone final point: take its track from the segment before it.
    return at(tr, i > s ? i - 1 : i, i, 1);
  }
  const j = i + 1;
  if ((tr.t[j] - tr.t[i]) * 100 > GAP_MS) return null;
  return at(tr, i, j, (tDs - tr.t[i]) / (tr.t[j] - tr.t[i]));
}

/** [lon, lat] path over the last `spanMs`, ending at the current position; stops at a gap. */
export function trailAt(tr: Tracks, p: [number, number], tMs: number, spanMs = 120_000): [number, number][] {
  const head = positionAt(tr, p, tMs);
  if (!head) return [];
  const [s, c] = p;
  const tDs = tMs / 100;
  const from = (tMs - spanMs) / 100;
  let i = floorIndex(tr.t, s, s + c, tDs);
  const out: [number, number][] = [[head.lon, head.lat]];
  let prev = tDs;
  while (i >= s && tr.t[i] > from && (prev - tr.t[i]) * 100 <= GAP_MS) {
    if (tr.t[i] !== tDs) out.push([tr.lon[i] / 1e5, tr.lat[i] / 1e5]);
    prev = tr.t[i];
    i--;
  }
  return out.reverse();
}
