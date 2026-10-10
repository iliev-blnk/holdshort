import type { DayJson, EventJ, FlightJ } from '../data/types';

const pad = (n: number) => String(n).padStart(2, '0');

/** ms since day start → "HH:MM" (24:00 at the very end of the day). */
export function hhmm(tMs: number): string {
  const min = Math.floor(tMs / 60_000);
  return `${pad(Math.floor(min / 60))}:${pad(min % 60)}`;
}

/** Seconds → "16.4 min", or "—" when unknown. */
export function minutes(s: number | null): string {
  return s === null ? '—' : `${(s / 60).toFixed(1)} min`;
}

/** Callsign, else the aircraft's hex code (the id is "<hex>-<first_ms>"). */
export function callsign(f: FlightJ): string {
  return f.cs ?? f.id.split('-')[0];
}

/** One line for the event feed, or null for events the feed doesn't show. */
export function eventText(e: EventJ, day: DayJson): string | null {
  const who = e.f === null ? '' : callsign(day.flights[e.f]);
  const rwy = e.rwy ? ` ${e.rwy}` : '';
  switch (e.kind) {
    case 'touchdown':
      return `${who} landed${rwy}`;
    case 'liftoff':
      return `${who} departed${rwy}`;
    case 'go_around':
      return `${who} go-around${rwy}`;
    case 'hold_start':
      return `${who} holding`;
    case 'config_change':
      return `Config ${e.detail || '—'}`;
    default:
      return null;
  }
}
