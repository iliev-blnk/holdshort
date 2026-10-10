import { expect, test } from 'vitest';
import { Clock, DAY_MS } from '../src/sim/clock';

test('plays at speed', () => {
  const ticks: number[] = [];
  const c = new Clock((t) => ticks.push(t), 21_600_000, 60);
  c.play();
  c.frame(0);
  c.frame(1000);
  expect(c.t).toBe(21_600_000 + 60_000);
  expect(ticks.at(-1)).toBe(c.t);
});

test('paused clock does not move', () => {
  const c = new Clock(() => {}, 1000, 60);
  c.frame(0);
  c.frame(5000);
  expect(c.t).toBe(1000);
  expect(c.playing).toBe(false);
});

test('a resumed clock does not jump over the pause', () => {
  const c = new Clock(() => {}, 0, 60);
  c.play();
  c.frame(0);
  c.frame(1000);
  c.pause();
  c.frame(50_000);
  c.play();
  c.frame(51_000);
  c.frame(52_000);
  expect(c.t).toBe(120_000);
});

test('seek clamps to the day', () => {
  const c = new Clock(() => {}, 0, 60);
  c.seek(-5);
  expect(c.t).toBe(0);
  c.seek(90_000_000);
  expect(c.t).toBe(DAY_MS);
});

test('stops at the end of the day', () => {
  const c = new Clock(() => {}, DAY_MS - 1000, 600);
  c.play();
  c.frame(0);
  c.frame(10_000);
  expect(c.t).toBe(DAY_MS);
  expect(c.playing).toBe(false);
});

test('speed change applies to the next frame', () => {
  const c = new Clock(() => {}, 0, 60);
  c.play();
  c.frame(0);
  c.setSpeed(600);
  c.frame(1000);
  expect(c.t).toBe(600_000);
  expect(c.speed).toBe(600);
});
