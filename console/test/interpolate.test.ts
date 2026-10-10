import { expect, test } from 'vitest';
import { positionAt, trailAt } from '../src/sim/interpolate';
import { hm, mk } from './helpers';

const P: [number, number] = [0, 2];

test('exact at a point', () => {
  const tr = mk([[hm(6, 0), 41.0, 28.0, 10], [hm(6, 1), 41.1, 28.2, 20]]);
  const p = positionAt(tr, P, hm(6, 0))!;
  expect([p.lat, p.lon, p.trk]).toEqual([41.0, 28.0, 10]);
});

test('linear between points', () => {
  const tr = mk([[hm(6, 0), 41.0, 28.0, 10], [hm(6, 1), 41.1, 28.2, 20]]);
  const p = positionAt(tr, P, hm(6, 0) + 30_000)!;
  expect(p.lat).toBeCloseTo(41.05, 6);
  expect(p.lon).toBeCloseTo(28.1, 6);
  expect(p.trk).toBeCloseTo(15, 6);
  expect(p.alt).toBe(5000);
});

test('track interpolates the short way round north', () => {
  const tr = mk([[hm(6, 0), 41.0, 28.0, 350], [hm(6, 1), 41.1, 28.0, 10]]);
  expect(positionAt(tr, P, hm(6, 0) + 30_000)!.trk).toBeCloseTo(0, 6);
});

test('hidden in a gap over 5 minutes', () => {
  const tr = mk([[hm(6, 0), 41.0, 28.0, 0], [hm(6, 6), 41.1, 28.0, 0]]);
  expect(positionAt(tr, P, hm(6, 3))).toBeNull();
});

test('hidden outside the flight', () => {
  const tr = mk([[hm(6, 0), 41.0, 28.0, 0], [hm(6, 1), 41.1, 28.0, 0]]);
  expect(positionAt(tr, P, hm(5, 59))).toBeNull();
  expect(positionAt(tr, P, hm(6, 1) + 1)).toBeNull();
  expect(positionAt(tr, P, hm(6, 1))).not.toBeNull();
});

test('interpolates across midnight', () => {
  const tr = mk([[-60_000, 41.0, 28.0, 0], [60_000, 41.1, 28.0, 0]]);
  expect(positionAt(tr, P, 0)!.lat).toBeCloseTo(41.05, 6);
});

test('missing track uses bearing', () => {
  const tr = mk([[hm(6, 0), 41.0, 28.0, null], [hm(6, 1), 41.0, 28.1, null]]);
  expect(positionAt(tr, P, hm(6, 0) + 30_000)!.trk).toBeCloseTo(90, 0);
});

test('trail covers two minutes and stops at a gap', () => {
  const rows: [number, number, number, number | null][] = [
    [hm(5, 50), 40.9, 28.0, 0], // before a 7-minute gap
    [hm(5, 57), 41.0, 28.0, 0],
    [hm(5, 58), 41.01, 28.0, 0],
    [hm(5, 59), 41.02, 28.0, 0],
    [hm(6, 0), 41.03, 28.0, 0],
  ];
  const tr = mk(rows);
  const full = trailAt(tr, [0, 5], hm(5, 59) + 30_000);
  // Points after 05:57:30 plus the interpolated head.
  expect(full.map((p) => p[1])).toEqual([41.01, 41.02, expect.closeTo(41.025, 6)]);
  const atGap = trailAt(tr, [0, 5], hm(5, 57) + 30_000, 600_000);
  expect(atGap.every((p) => p[1] >= 41.0)).toBe(true);
});
