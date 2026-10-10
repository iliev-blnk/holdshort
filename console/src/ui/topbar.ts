import type { DayIndex } from '../data/types';

const REPO = 'https://github.com/iliev-blnk/holdshort';

/** 2026-10-02 → "2 Oct 2026" (UTC). */
export function longDate(date: string): string {
  return new Date(`${date}T00:00:00Z`).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' });
}

/** 2026-10-02 → "Fri 2 Oct" (UTC). */
function shortDate(date: string): string {
  return new Date(`${date}T00:00:00Z`).toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', timeZone: 'UTC' });
}

export function mountTopbar(el: HTMLElement, index: DayIndex, current: string, onDay: (date: string) => void) {
  el.innerHTML = `
    <span class="brand">holdshort</span>
    <span class="caption"></span>
    <select class="day" aria-label="Day">
      ${index.days.map((d) => `<option value="${d.date}">${shortDate(d.date)}</option>`).join('')}
    </select>
    <a class="gh" href="${REPO}" target="_blank" rel="noopener">GitHub</a>`;
  const caption = el.querySelector<HTMLSpanElement>('.caption')!;
  const select = el.querySelector<HTMLSelectElement>('.day')!;
  const setDay = (date: string) => {
    select.value = date;
    caption.textContent = `Istanbul Airport, rebuilt from open ADS-B data · ${longDate(date)}`;
  };
  select.addEventListener('change', () => onDay(select.value));
  setDay(current);
  return { setDay };
}
