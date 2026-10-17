import { expect, test } from 'vitest';
import { cardHtml } from '../src/ui/flightcard';
import { fixture } from './helpers';

test('card shows the landing time predicted at 40 nm and its error', () => {
  const { day } = fixture();
  // THY1: touchdown 06:12:20, ASMA 900 s → entered 40 nm at 05:57:20.
  const f = { ...day.flights[0], eta_s: 936 };
  const html = cardHtml(f);
  expect(html).toContain('Predicted at 40 nm');
  // 05:57:20 + 15.6 min = 06:12:56.
  expect(html).toContain('06:12 UTC (15.6 min)');
  expect(html).toContain('+0.6 min');
});

test('no prediction row without a prediction', () => {
  const { day } = fixture();
  expect(cardHtml({ ...day.flights[0], eta_s: null })).not.toContain('Predicted');
  expect(cardHtml({ ...day.flights[1], eta_s: null })).not.toContain('Predicted');
});

test('an early prediction shows a negative error', () => {
  const { day } = fixture();
  expect(cardHtml({ ...day.flights[0], eta_s: 852 })).toContain('−0.8 min');
});

test('a flagged approach shows on the card', () => {
  const { day } = fixture();
  const html = cardHtml(day.flights[0]);
  expect(html).toContain('<dt>Approach</dt>');
  expect(html).toContain('Unusual (top 1%) · +18 nm flown (typical +4)');
});

test('no approach row without a score', () => {
  const { day } = fixture();
  expect(cardHtml({ ...day.flights[0], ap: null })).not.toContain('Approach');
  expect(cardHtml(day.flights[1])).not.toContain('Approach');
});
