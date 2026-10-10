import type { FlightJ } from '../data/types';
import { callsign, hhmm, minutes } from './format';

const KIND: Record<FlightJ['kind'], string> = {
  arrival: 'Arrival',
  departure: 'Departure',
  both: 'Arrival and departure',
  overflight: 'Overflight',
  other: 'Other traffic',
};

const row = (k: string, v: string) => `<dt>${k}</dt><dd>${v}</dd>`;

export function mountFlightCard(el: HTMLElement, onClose: () => void) {
  el.hidden = true;
  el.addEventListener('click', (e) => {
    if ((e.target as HTMLElement).closest('.close')) onClose();
  });
  return {
    show(f: FlightJ | null) {
      el.hidden = f === null;
      if (!f) return;
      const est = f.low ? ' <span class="badge">low confidence</span>' : '';
      const rows = [row('Kind', KIND[f.kind])];
      if (f.td !== null) rows.push(row('Landed', `${hhmm(f.td)} UTC${f.arr_rwy ? ` · ${f.arr_rwy}` : ''}${est}`));
      if (f.lo !== null) rows.push(row('Departed', `${hhmm(f.lo)} UTC${f.dep_rwy ? ` · ${f.dep_rwy}` : ' · runway unknown'}${est}`));
      if (f.asma_s !== null) rows.push(row('ASMA', minutes(f.asma_s)));
      if (f.holds) rows.push(row('Holding', `${f.holds} × · ${minutes(f.hold_s)}`));
      if (f.ga) rows.push(row('Go-arounds', String(f.ga)));
      el.innerHTML = `
        <button class="close" aria-label="Close">×</button>
        <div class="cs">${callsign(f)}</div>
        <div class="sub">${[f.type, f.reg].filter(Boolean).join(' · ') || '&nbsp;'}</div>
        <dl>${rows.join('')}</dl>`;
    },
  };
}
