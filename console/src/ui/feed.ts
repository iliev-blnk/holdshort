import type { DayJson, EventJ } from '../data/types';
import { T } from '../theme';
import { eventText, hhmm } from './format';

const DOT: Record<string, string> = { touchdown: T.arr, liftoff: T.dep, go_around: T.text, hold_start: T.muted, config_change: T.muted, unusual_approach: T.anom };

/** Escape text for innerHTML: callsigns, registrations and types come from third-party data. */
const ENTITY: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

export function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ENTITY[c]);
}

/**
 * How the feed must change: nothing, a full rebuild (another day, or time went
 * back), or `added` new events at the top (time moved forward).
 */
export function feedChange(shown: { day: DayJson | null; count: number }, day: DayJson, count: number): 'none' | 'rebuild' | { added: number } {
  if (shown.day !== day || count < shown.count) return 'rebuild';
  if (count === shown.count) return 'none';
  return { added: count - shown.count };
}

/** One feed row, or null for events the feed doesn't show. */
export function feedRow(i: number, e: EventJ, day: DayJson): string | null {
  const text = eventText(e, day);
  if (text === null) return null;
  const low = e.f !== null && day.flights[e.f].low && (e.kind === 'touchdown' || e.kind === 'liftoff');
  return `<li data-i="${i}"><i style="background:${DOT[e.kind]}"></i><time>${hhmm(e.t)}</time><span>${esc(text)}${low ? ' <em>est.</em>' : ''}</span></li>`;
}
