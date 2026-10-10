import type { DayJson } from '../data/types';

export interface UrlState {
  d?: string;
  t?: number;
  f?: string;
}

/** Read ?d=YYYY-MM-DD&t=HH:MM&f=<flight id>; values that don't make sense are dropped. */
export function readUrl(search: string, days: string[]): UrlState {
  const q = new URLSearchParams(search);
  const out: UrlState = {};
  const d = q.get('d');
  if (d && days.includes(d)) out.d = d;
  const m = /^(\d{2}):(\d{2})$/.exec(q.get('t') ?? '');
  if (m && +m[1] < 24 && +m[2] < 60) out.t = (+m[1] * 60 + +m[2]) * 60_000;
  const f = q.get('f');
  if (f) out.f = f;
  return out;
}

const pad = (n: number) => String(n).padStart(2, '0');

export function writeUrl(s: { d: string; t: number; f?: string | null }): string {
  const min = Math.floor(s.t / 60_000);
  const hh = Math.min(Math.floor(min / 60), 23);
  const mm = hh === 23 && min >= 24 * 60 ? 59 : min % 60;
  const q = `?d=${s.d}&t=${pad(hh)}:${pad(mm)}`;
  return s.f ? `${q}&f=${encodeURIComponent(s.f)}` : q;
}

/** Index of the flight with this id, or null when there is none (e.g. a link from another day). */
export function selectFromUrl(day: DayJson, f: string | undefined): number | null {
  if (!f) return null;
  const i = day.flights.findIndex((fl) => fl.id === f);
  return i < 0 ? null : i;
}
