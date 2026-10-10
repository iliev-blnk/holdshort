import type { DayJson, Kind, Tracks } from '../data/types';
import { type Pos, positionAt } from './interpolate';

export interface SimState {
  aircraft: { f: number; kind: Kind; pos: Pos }[];
  /** Runway configuration label in use ('' before the first one). */
  config: string;
  /** Indices into day.events with t <= now, newest first. */
  events: number[];
  /** Touchdowns plus liftoffs in the last hour. */
  lastHour: number;
  /** Median ASMA of the arrivals so far. */
  asmaMedianS: number | null;
}

const HOUR_MS = 3_600_000;
const byTime = new WeakMap<DayJson, number[]>();

function eventsByTime(day: DayJson): number[] {
  let order = byTime.get(day);
  if (!order) {
    order = day.events.map((_, i) => i).sort((a, b) => day.events[a].t - day.events[b].t);
    byTime.set(day, order);
  }
  return order;
}

export function stateAt(day: DayJson, tr: Tracks, tMs: number): SimState {
  const tDs = tMs / 100;
  const aircraft: SimState['aircraft'] = [];
  let lastHour = 0;
  const asma: number[] = [];
  const inHour = (t: number | null) => t !== null && t > tMs - HOUR_MS && t <= tMs;
  day.flights.forEach((fl, f) => {
    const [s, c] = fl.p;
    if (c > 0 && tr.t[s] <= tDs && tr.t[s + c - 1] >= tDs) {
      const pos = positionAt(tr, fl.p, tMs);
      if (pos) aircraft.push({ f, kind: fl.kind, pos });
    }
    if (inHour(fl.td)) lastHour++;
    if (inHour(fl.lo)) lastHour++;
    if (fl.td !== null && fl.td <= tMs && fl.asma_s !== null) asma.push(fl.asma_s);
  });

  const order = eventsByTime(day);
  let n = 0;
  while (n < order.length && day.events[order[n]].t <= tMs) n++;
  const events = order.slice(0, n).reverse();

  let config = '';
  for (const c of day.config) {
    if (c.t > tMs) break;
    config = c.label;
  }

  asma.sort((a, b) => a - b);
  return { aircraft, config, events, lastHour, asmaMedianS: asma.length ? asma[asma.length >> 1] : null };
}
