import { expect, test } from 'vitest';
import type { ApJ } from '../src/data/types';
import { approachRow, ordinal, reasonText } from '../src/ui/approach';

const ap = (o: Partial<ApJ>): ApJ => ({ pct: null, flag: false, reasons: [], unscored: null, ...o });

test('ordinals', () => {
  expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 54, 100, 101, 111].map(ordinal)).toEqual([
    '1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '23rd', '54th', '100th', '101st', '111th',
  ]);
});

test('a scored approach shows its percentile', () => {
  expect(approachRow(ap({ pct: 54.9 }))).toBe('54th percentile');
  expect(approachRow(ap({ pct: 1.2 }))).toBe('1st percentile');
  expect(approachRow(ap({ pct: 0 }))).toBe('0th percentile');
  expect(approachRow(ap({ pct: 100 }))).toBe('100th percentile');
});

test('reason texts', () => {
  expect(reasonText({ m: 'extra_nm', v: 18.4, typ: 4.1 })).toBe('+18 nm flown (typical +4)');
  expect(reasonText({ m: 'time_min', v: 31.2, typ: 16.4 })).toBe('31 min from 40 nm (typical 16)');
  expect(reasonText({ m: 'turn_deg', v: 540.2, typ: 120 })).toBe('540° of turns (typical 120°)');
  expect(reasonText({ m: 'level_min', v: 9.4, typ: 2.1 })).toBe('9 min level below 10,000 ft (typical 2)');
  expect(reasonText({ m: 'high_10nm_ft', v: -230, typ: -508.9 })).toBe('280 ft higher than usual at 10 nm');
  expect(reasonText({ m: 'high_10nm_ft', v: -996, typ: 40.3 })).toBe('1,040 ft lower than usual at 10 nm');
  expect(reasonText({ m: 'gs_4nm_kt', v: 190, typ: 150.2 })).toBe('190 kt at 4 nm (typical 150)');
  expect(reasonText({ m: 'xtk_final_m', v: 600.4, typ: 80 })).toBe('600 m off the centreline on final (typical 80)');
});

test('a flagged approach shows its reasons, or the path-shape fallback', () => {
  const r = [{ m: 'extra_nm', v: 18, typ: 4 }, { m: 'time_min', v: 31, typ: 16 }];
  expect(approachRow(ap({ pct: 99.7, flag: true, reasons: r }))).toBe(
    'Unusual (top 1%) · +18 nm flown (typical +4) · 31 min from 40 nm (typical 16)',
  );
  expect(approachRow(ap({ pct: 99.2, flag: true }))).toBe('Unusual (top 1%) · unusual path shape');
});

test('unscored approaches say why', () => {
  expect(approachRow(ap({ unscored: 'lost_on_final' }))).toBe('not scored (lost on final)');
  expect(approachRow(ap({ unscored: 'coverage' }))).toBe('not scored (coverage gaps)');
  expect(approachRow(ap({ unscored: 'runway' }))).toBe('not scored (no reference for this runway)');
  expect(approachRow(ap({ unscored: 'no_entry' }))).toBe('not scored (no 40 nm entry)');
  expect(approachRow(ap({ unscored: 'other' }))).toBe('not scored');
});

test('an unknown reason key is escaped', () => {
  expect(reasonText({ m: '<b>', v: 1, typ: 0 })).toBe('&lt;b&gt;');
});
