import type { DayJson } from '../data/types';
import type { SimState } from '../sim/state';
import { T } from '../theme';
import { eventText, hhmm, minutes } from './format';

const MAX_ROWS = 200;
const DOT: Record<string, string> = { touchdown: T.arr, liftoff: T.dep, go_around: T.text, hold_start: T.muted, config_change: T.muted };

export interface Panel {
  update(state: SimState, day: DayJson, tMs: number): void;
}

export function mountPanel(el: HTMLElement, onEvent: (i: number) => void): Panel {
  el.innerHTML = `
    <button class="now" aria-expanded="false">
      <span class="label">Now · <span class="clock">--:--</span> UTC</span>
      <span class="config">—</span>
    </button>
    <dl class="stats">
      <dt>Movements, last hour</dt><dd class="s-hour">0</dd>
      <dt>Median ASMA today</dt><dd class="s-asma">—</dd>
      <dt>Go-arounds today</dt><dd class="s-ga">0</dd>
    </dl>
    <div class="label feed-label">Events</div>
    <ol class="feed"></ol>
    <p class="note">Departures appear only once they climb into coverage, and there is no ground coverage. Estimated times are marked.</p>`;
  const $ = <E extends Element>(s: string) => el.querySelector<E>(s)!;
  const now = $<HTMLButtonElement>('.now');
  const clock = $<HTMLSpanElement>('.clock');
  const config = $<HTMLSpanElement>('.config');
  const sHour = $<HTMLElement>('.s-hour');
  const sAsma = $<HTMLElement>('.s-asma');
  const sGa = $<HTMLElement>('.s-ga');
  const feed = $<HTMLOListElement>('.feed');

  // On phones the panel is a bottom sheet: the "Now" block toggles it open.
  now.addEventListener('click', () => {
    const open = el.classList.toggle('open');
    now.setAttribute('aria-expanded', String(open));
  });
  feed.addEventListener('click', (e) => {
    const li = (e.target as HTMLElement).closest<HTMLLIElement>('li[data-i]');
    if (li) onEvent(Number(li.dataset.i));
  });

  let shownDay: DayJson | null = null;
  let shownCount = -1;
  let shownMinute = -1;

  return {
    update(state, day, tMs) {
      const minute = Math.floor(tMs / 60_000);
      if (minute !== shownMinute) {
        shownMinute = minute;
        clock.textContent = hhmm(tMs);
      }
      config.textContent = state.config || '—';
      sHour.textContent = String(state.lastHour);
      sAsma.textContent = minutes(state.asmaMedianS);
      if (day === shownDay && state.events.length === shownCount) return;
      shownDay = day;
      shownCount = state.events.length;
      let ga = 0;
      const rows: string[] = [];
      for (const i of state.events) {
        const e = day.events[i];
        if (e.kind === 'go_around') ga++;
        if (rows.length >= MAX_ROWS) continue;
        const text = eventText(e, day);
        if (text === null) continue;
        const low = e.f !== null && day.flights[e.f].low && (e.kind === 'touchdown' || e.kind === 'liftoff');
        rows.push(
          `<li data-i="${i}"><i style="background:${DOT[e.kind]}"></i><time>${hhmm(e.t)}</time><span>${text}${low ? ' <em>est.</em>' : ''}</span></li>`,
        );
      }
      sGa.textContent = String(ga);
      feed.innerHTML = rows.join('');
    },
  };
}
