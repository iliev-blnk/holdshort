import type { FlightJ } from '../data/types';
import { esc } from './feed';
import { callsign, hhmm, minutes } from './format';

const KIND: Record<FlightJ['kind'], string> = {
  arrival: 'Arrival',
  departure: 'Departure',
  both: 'Arrival and departure',
  overflight: 'Overflight',
  other: 'Other traffic',
};

const row = (k: string, v: string) => `<dt>${k}</dt><dd>${v}</dd>`;

/** The card's inner HTML; every data string is escaped. */
export function cardHtml(f: FlightJ): string {
  const est = f.low ? ' <span class="badge">low confidence</span>' : '';
  const rows = [row('Kind', KIND[f.kind])];
  if (f.td !== null) rows.push(row('Landed', `${hhmm(f.td)} UTC${f.arr_rwy ? ` · ${esc(f.arr_rwy)}` : ''}${est}`));
  if (f.lo !== null) rows.push(row('Departed', `${hhmm(f.lo)} UTC${f.dep_rwy ? ` · ${esc(f.dep_rwy)}` : ' · runway unknown'}${est}`));
  if (f.asma_s !== null) rows.push(row('ASMA', minutes(f.asma_s)));
  if (f.eta_s !== null && f.td !== null && f.asma_s !== null) {
    // The model ran when the flight crossed 40 nm, i.e. touchdown − ASMA.
    const entry = f.td - f.asma_s * 1000;
    const err = (f.eta_s - f.asma_s) / 60;
    const signed = `${err < 0 ? '−' : '+'}${Math.abs(err).toFixed(1)} min`;
    rows.push(row('Predicted at 40 nm', `${hhmm(entry + f.eta_s * 1000)} UTC (${(f.eta_s / 60).toFixed(1)} min) · ${signed}`));
  }
  if (f.holds) rows.push(row('Holding', `${f.holds} × · ${minutes(f.hold_s)}`));
  if (f.ga) rows.push(row('Go-arounds', String(f.ga)));
  const sub = [f.type, f.reg].filter((x): x is string => !!x).map(esc).join(' · ') || '&nbsp;';
  return `
    <button class="close" aria-label="Close">×</button>
    <div class="cs">${esc(callsign(f))}</div>
    <div class="sub">${sub}</div>
    <dl>${rows.join('')}</dl>`;
}

export function mountFlightCard(el: HTMLElement, onClose: () => void) {
  el.hidden = true;
  el.addEventListener('click', (e) => {
    if ((e.target as HTMLElement).closest('.close')) onClose();
  });
  return {
    show(f: FlightJ | null) {
      el.hidden = f === null;
      if (f) el.innerHTML = cardHtml(f);
    },
  };
}
