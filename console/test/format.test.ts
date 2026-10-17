import { expect, test } from 'vitest';
import type { EventJ } from '../src/data/types';
import { eventText, hhmm, minutes } from '../src/ui/format';
import { fixture } from './helpers';

const ev = (kind: EventJ['kind'], f: number | null, rwy: string | null = null, detail: string | null = null): EventJ => ({
  t: 0,
  kind,
  f,
  rwy,
  lat: null,
  lon: null,
  detail,
});

test('hhmm', () => {
  expect(hhmm(0)).toBe('00:00');
  expect(hhmm(23_400_000)).toBe('06:30');
  expect(hhmm(23_459_999)).toBe('06:30');
  expect(hhmm(86_400_000)).toBe('24:00');
});

test('eventText for the fixture events', () => {
  const { day } = fixture();
  const texts = day.events.map((e) => eventText(e, day));
  expect(texts).toEqual(['PGT2 departed', 'Config ARR 35R / DEP -', 'THY1 landed 35R', 'THY1 unusual approach 35R']);
});

test('eventText for go-arounds and holds', () => {
  const { day } = fixture();
  expect(eventText(ev('go_around', 0, '35R'), day)).toBe('THY1 go-around 35R');
  expect(eventText(ev('hold_start', 2), day)).toBe('UAE3 holding');
});

test('eventText hides asma_entry and hold_end', () => {
  const { day } = fixture();
  expect(eventText(ev('asma_entry', 0), day)).toBeNull();
  expect(eventText(ev('hold_end', 0), day)).toBeNull();
});

test('eventText falls back to the aircraft hex without a callsign', () => {
  const { day } = fixture();
  const d = structuredClone(day);
  d.flights[0].cs = null;
  expect(eventText(ev('touchdown', 0, '35R'), d)).toBe('4bb208 landed 35R');
});

test('minutes', () => {
  expect(minutes(null)).toBe('—');
  expect(minutes(984)).toBe('16.4 min');
});
