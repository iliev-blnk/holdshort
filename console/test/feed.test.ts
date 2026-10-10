import { expect, test } from 'vitest';
import type { DayJson, EventJ } from '../src/data/types';
import { feedChange, feedRow } from '../src/ui/feed';
import { cardHtml } from '../src/ui/flightcard';
import { fixture } from './helpers';

test('feed grows in place while time moves forward on the same day', () => {
  const { day } = fixture();
  expect(feedChange({ day, count: 1 }, day, 3)).toEqual({ added: 2 });
  expect(feedChange({ day, count: 3 }, day, 3)).toBe('none');
});

test('feed rebuilds on a backward seek or another day', () => {
  const { day } = fixture();
  const other: DayJson = structuredClone(day);
  expect(feedChange({ day, count: 3 }, day, 1)).toBe('rebuild');
  expect(feedChange({ day, count: 3 }, other, 3)).toBe('rebuild');
  expect(feedChange({ day: null, count: 0 }, day, 0)).toBe('rebuild');
});

test('feed rows escape data from the aircraft database', () => {
  const { day } = fixture();
  const d: DayJson = structuredClone(day);
  d.flights[0].cs = '<img src=x onerror=alert(1)>';
  const e: EventJ = { ...d.events[2] };
  const html = feedRow(2, e, d)!;
  expect(html).not.toContain('<img');
  expect(html).toContain('&lt;img');
});

test('the flight card escapes data from the aircraft database', () => {
  const { day } = fixture();
  const f = { ...day.flights[0], reg: '<b>TC</b>', type: 'A321"><script>', arr_rwy: '<i>35R' };
  const html = cardHtml(f);
  expect(html).not.toMatch(/<b>|<script>|<i>/);
  expect(html).toContain('&lt;b&gt;TC&lt;/b&gt;');
});
