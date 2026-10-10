import { expect, test } from 'vitest';
import { msToX, xToMs } from '../src/ui/timeline';

const PAD = { l: 10, r: 20 };

test('maps the day onto the padded width', () => {
  expect(xToMs(10, 1030, PAD)).toBe(0);
  expect(xToMs(1010, 1030, PAD)).toBe(86_400_000);
  expect(xToMs(510, 1030, PAD)).toBe(43_200_000);
  expect(msToX(43_200_000, 1030, PAD)).toBe(510);
});

test('round-trips', () => {
  for (const t of [0, 36_000_000, 86_400_000]) expect(xToMs(msToX(t, 800, PAD), 800, PAD)).toBeCloseTo(t, 3);
});

test('clamps outside the padding', () => {
  expect(xToMs(-50, 1030, PAD)).toBe(0);
  expect(xToMs(5000, 1030, PAD)).toBe(86_400_000);
});
