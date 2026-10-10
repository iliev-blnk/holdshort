import { expect, test } from 'vitest';
import { stateAt } from '../src/sim/state';
import { fixture, hm } from './helpers';

test('stateAt on the fixture at 06:15', () => {
  const { day, tracks } = fixture();
  const s = stateAt(day, tracks, hm(6, 15));
  // THY1 landed at 06:12; PGT2 and UAE3 are airborne.
  expect(s.aircraft.map((a) => day.flights[a.f].cs).sort()).toEqual(['PGT2', 'UAE3']);
  expect(s.aircraft.find((a) => day.flights[a.f].cs === 'PGT2')!.kind).toBe('departure');
  expect(s.config).toBe('ARR 35R / DEP -');
  expect(s.events.map((i) => day.events[i].kind)).toEqual(['touchdown', 'config_change', 'liftoff']);
  expect(s.lastHour).toBe(2);
  expect(s.asmaMedianS).toBe(900);
});

test('stateAt before any config', () => {
  const { day, tracks } = fixture();
  const s = stateAt(day, tracks, hm(6, 5));
  expect(s.config).toBe('');
  expect(s.events).toEqual([]);
  expect(s.asmaMedianS).toBeNull();
  expect(s.aircraft.map((a) => day.flights[a.f].cs).sort()).toEqual(['THY1', 'UAE3']);
});
