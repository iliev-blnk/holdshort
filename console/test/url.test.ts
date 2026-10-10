import { expect, test } from 'vitest';
import { readUrl, selectFromUrl, writeUrl } from '../src/ui/url';
import { fixture } from './helpers';

const DAYS = ['2026-10-01', '2026-10-02'];

test('readUrl reads day, time and flight', () => {
  expect(readUrl('?d=2026-10-01&t=06:30&f=4bb208-1', DAYS)).toEqual({ d: '2026-10-01', t: 23_400_000, f: '4bb208-1' });
});

test('readUrl ignores bad values', () => {
  expect(readUrl('?d=2026-12-25&t=25:99', DAYS)).toEqual({});
  expect(readUrl('?t=abc&f=', DAYS)).toEqual({});
  expect(readUrl('', DAYS)).toEqual({});
});

test('url round-trips', () => {
  const s = writeUrl({ d: '2026-10-02', t: 23_430_000, f: '4ba9c1-1' });
  expect(s).toBe('?d=2026-10-02&t=06:30&f=4ba9c1-1');
  expect(readUrl(s, DAYS)).toEqual({ d: '2026-10-02', t: 23_400_000, f: '4ba9c1-1' });
  expect(writeUrl({ d: '2026-10-02', t: 0, f: null })).toBe('?d=2026-10-02&t=00:00');
});

test('flight selection from the URL', () => {
  const { day } = fixture();
  expect(selectFromUrl(day, '4ba9c1-1')).toBe(1);
  expect(selectFromUrl(day, 'nope')).toBeNull();
  expect(selectFromUrl(day, undefined)).toBeNull();
});
