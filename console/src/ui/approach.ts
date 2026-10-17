import type { ApJ, ReasonJ } from '../data/types';
import { esc } from './feed';

/** 1st, 2nd, 3rd, 4th, …, 11th–13th, 21st, … */
export function ordinal(n: number): string {
  const t = n % 100;
  const s = t >= 11 && t <= 13 ? 'th' : (['th', 'st', 'nd', 'rd'][n % 10] ?? 'th');
  return `${n}${s}`;
}

const num = (v: number) => Math.round(v).toLocaleString('en-US');
const signed = (v: number) => `${v < 0 ? '−' : '+'}${num(Math.abs(v))}`;

/** One reason as text; the 10 nm height is against the usual height, not the 3° path. */
export function reasonText(r: ReasonJ): string {
  switch (r.m) {
    case 'extra_nm':
      return `${signed(r.v)} nm flown (typical ${signed(r.typ)})`;
    case 'time_min':
      return `${num(r.v)} min from 40 nm (typical ${num(r.typ)})`;
    case 'turn_deg':
      return `${num(r.v)}° of turns (typical ${num(r.typ)}°)`;
    case 'level_min':
      return `${num(r.v)} min level below 10,000 ft (typical ${num(r.typ)})`;
    case 'high_10nm_ft': {
      const d = Math.round((r.v - r.typ) / 10) * 10;
      return `${num(Math.abs(d))} ft ${d >= 0 ? 'higher' : 'lower'} than usual at 10 nm`;
    }
    case 'gs_4nm_kt':
      return `${num(r.v)} kt at 4 nm (typical ${num(r.typ)})`;
    case 'xtk_final_m':
      return `${num(r.v)} m off the centreline on final (typical ${num(r.typ)})`;
    default:
      return esc(r.m);
  }
}

const UNSCORED: Record<string, string> = {
  lost_on_final: 'lost on final',
  coverage: 'coverage gaps',
  runway: 'no reference for this runway',
  no_entry: 'no 40 nm entry',
};

/** The flight card's Approach value (HTML-safe). */
export function approachRow(ap: ApJ): string {
  if (ap.unscored !== null || ap.pct === null) {
    const why = ap.unscored === null ? undefined : UNSCORED[ap.unscored];
    return why ? `not scored (${why})` : 'not scored';
  }
  if (ap.flag) {
    const why = ap.reasons.length ? ap.reasons.map(reasonText) : ['unusual path shape'];
    return ['Unusual (top 1%)', ...why].join(' · ');
  }
  return `${ordinal(Math.floor(ap.pct))} percentile`;
}
