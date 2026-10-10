import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import { altAt, decodeTracks, gsAt, lonLatAt, trkAt } from '../src/data/decode';
import type { DayJson } from '../src/data/types';

const dir = new URL('./fixture/LTFM/2026-10-02/', import.meta.url);
const buf = () => {
  const b = readFileSync(new URL('tracks.bin', dir));
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer;
};
const day: DayJson = JSON.parse(readFileSync(new URL('day.json', dir), 'utf8'));

/** A one-point buffer with chosen raw column values. */
function onePoint(alt: number, trk: number, gs: number): ArrayBuffer {
  const b = new ArrayBuffer(24 + 18);
  const v = new DataView(b);
  'HSTK'.split('').forEach((c, i) => v.setUint8(i, c.charCodeAt(0)));
  v.setUint32(4, 1, true);
  v.setUint32(8, 1, true);
  v.setBigInt64(16, 0n, true);
  v.setInt32(24, 600, true);
  v.setInt32(28, 4_100_000, true);
  v.setInt32(32, 2_870_000, true);
  v.setInt16(36, alt, true);
  v.setUint16(38, trk, true);
  v.setUint16(40, gs, true);
  return b;
}

describe('decodeTracks', () => {
  test('decodes the fixture', () => {
    const tr = decodeTracks(buf());
    expect(tr.n).toBe(32);
    expect(tr.dayStartMs).toBe(1_790_899_200_000);
    const [start, count] = day.flights[0].p;
    expect(count).toBe(11);
    // THY1's first point is at 06:02.
    expect(tr.t[start]).toBe((6 * 60 + 2) * 600);
    expect(trkAt(tr, start)).toBe(359);
    expect(gsAt(tr, start)).toBe(150);
    const [lon, lat] = lonLatAt(tr, start);
    expect(lon).toBeCloseTo(28.72776, 5);
    expect(lat).toBeCloseTo(41.26192 - 10 / 60, 5);
  });

  test('rejects bad magic', () => {
    const b = buf();
    new Uint8Array(b)[0] = 'X'.charCodeAt(0);
    expect(() => decodeTracks(b)).toThrow('bad magic');
  });

  test('rejects unknown version', () => {
    const b = buf();
    new DataView(b).setUint32(4, 2, true);
    expect(() => decodeTracks(b)).toThrow('unsupported version 2');
  });

  test('decodes missing markers as null', () => {
    const missing = decodeTracks(onePoint(-32768, 65535, 65535));
    expect(altAt(missing, 0)).toBeNull();
    expect(trkAt(missing, 0)).toBeNull();
    expect(gsAt(missing, 0)).toBeNull();
    const present = decodeTracks(onePoint(5000, 9000, 1405));
    expect(altAt(present, 0)).toBe(5000);
    expect(trkAt(present, 0)).toBe(90);
    expect(gsAt(present, 0)).toBe(140.5);
  });
});
