import type { DayJson } from '../data/types';
import type { SimState } from '../sim/state';
import { feedChange, feedRow } from './feed';
import { hhmm, minutes } from './format';

const MAX_ROWS = 200;

export interface Panel {
  update(state: SimState, day: DayJson, tMs: number): void;
  /** Show nothing (e.g. while no day is loaded). */
  clear(): void;
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
  // pointerdown, not click: rows shift as new events arrive at high speed,
  // so a press and its release can land on different rows.
  feed.addEventListener('pointerdown', (e) => {
    const li = (e.target as HTMLElement).closest<HTMLLIElement>('li[data-i]');
    if (li) onEvent(Number(li.dataset.i));
  });

  let shownMinute = -1;
  let shown: { day: DayJson | null; count: number } = { day: null, count: 0 };
  let ga = 0;

  const rowsFor = (events: number[], day: DayJson) => {
    const rows: string[] = [];
    for (const i of events) {
      if (rows.length >= MAX_ROWS) break;
      const r = feedRow(i, day.events[i], day);
      if (r !== null) rows.push(r);
    }
    return rows.join('');
  };
  const countGa = (events: number[], day: DayJson) => events.filter((i) => day.events[i].kind === 'go_around').length;

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
      const change = feedChange(shown, day, state.events.length);
      if (change === 'none') return;
      if (change === 'rebuild') {
        feed.innerHTML = rowsFor(state.events, day);
        ga = countGa(state.events, day);
      } else {
        const fresh = state.events.slice(0, change.added);
        const html = rowsFor(fresh, day);
        if (html) {
          feed.insertAdjacentHTML('afterbegin', html);
          while (feed.children.length > MAX_ROWS) feed.lastElementChild!.remove();
        }
        ga += countGa(fresh, day);
      }
      sGa.textContent = String(ga);
      shown = { day, count: state.events.length };
    },
    clear() {
      shown = { day: null, count: 0 };
      ga = 0;
      shownMinute = -1;
      feed.innerHTML = '';
      config.textContent = '—';
      sHour.textContent = '0';
      sAsma.textContent = minutes(null);
      sGa.textContent = '0';
    },
  };
}
