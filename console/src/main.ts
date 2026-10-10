import '@fontsource/inter/latin-400.css';
import '@fontsource/inter/latin-600.css';
import '@fontsource/jetbrains-mono/latin-400.css';
import { MapboxOverlay } from '@deck.gl/mapbox';
import { latestOnly, loadDay, loadIndex } from './data/load';
import type { DayJson, Tracks } from './data/types';
import { buildLayers } from './map/layers';
import { createMap } from './map/map';
import { Clock } from './sim/clock';
import { stateAt } from './sim/state';
import './style.css';
import { mountFlightCard } from './ui/flightcard';
import { mountPanel } from './ui/panel';
import { mountTimeline } from './ui/timeline';
import { mountTopbar } from './ui/topbar';
import { readUrl, selectFromUrl, writeUrl } from './ui/url';

const DATA = import.meta.env.VITE_DATA_BASE ?? 'data';
const BASEMAP = import.meta.env.VITE_BASEMAP ?? 'basemap/tiles.json';
const DEFAULT_DAY = '2026-10-02';
/** 10:00 UTC: the busiest hour in coverage (about 32 aircraft in view). */
const DEFAULT_START_MS = 36_000_000;
const REPO = 'https://github.com/iliev-blnk/holdshort';
const URL_EVERY_MS = 500;

function fullPageMessage(text: string) {
  document.querySelector('#app')!.innerHTML = `<div class="message"><p>${text}</p><p><a href="${REPO}#console">See the README on GitHub</a></p></div>`;
}

function hasWebGL(): boolean {
  try {
    return !!document.createElement('canvas').getContext('webgl2');
  } catch {
    return false;
  }
}

async function main() {
  if (!hasWebGL()) return fullPageMessage('This replay needs WebGL, which this browser does not offer.');
  const app = document.querySelector<HTMLDivElement>('#app')!;
  let index;
  try {
    index = await loadIndex(DATA);
  } catch {
    return fullPageMessage('No data found. The README explains how to build it.');
  }
  app.innerHTML = `
    <header id="top"></header>
    <div id="map"><div id="card" class="card"></div><div id="notice" class="notice" hidden></div></div>
    <aside id="panel"></aside>
    <footer id="timeline"></footer>`;
  const airport = index.airport;
  const days = index.days.map((d) => d.date);
  const url = readUrl(location.search, days);
  let date = url.d ?? (days.includes(DEFAULT_DAY) ? DEFAULT_DAY : days[0]);
  const notice = document.getElementById('notice')!;
  const showNotice = (text: string | null) => {
    notice.hidden = text === null;
    notice.textContent = text ?? '';
  };

  // The map and the day's data load in parallel.
  const mapReady = createMap(document.getElementById('map')!, BASEMAP);
  let loaded: { day: DayJson; tracks: Tracks } | null = null;
  try {
    loaded = await loadDay(DATA, airport, date);
  } catch {
    showNotice('This day could not be loaded.');
  }
  let day: DayJson | null = loaded?.day ?? null;
  let tracks: Tracks | null = loaded?.tracks ?? null;

  const map = await mapReady;
  const overlay = new MapboxOverlay({ interleaved: false, layers: [] });
  map.addControl(overlay);

  let selected: number | null = day ? selectFromUrl(day, url.f) : null;
  const clock = new Clock(() => {}, url.t ?? DEFAULT_START_MS, 60);
  const card = mountFlightCard(document.getElementById('card')!, () => select(null));
  function select(f: number | null) {
    selected = f;
    card.show(f !== null && day ? day.flights[f] : null);
  }
  select(selected);

  const empty: DayJson = { v: 1, airport, date, day_start_ms: 0, flights: [], events: [], config: [], hist: { bin_s: 600, arr: [], dep: [] } };
  const timeline = mountTimeline(document.getElementById('timeline')!, clock, day ?? empty);
  const panel = mountPanel(document.getElementById('panel')!, (i) => {
    if (!day) return;
    const e = day.events[i];
    clock.seek(e.t);
    if (e.f !== null) select(e.f);
  });
  const loadLatest = latestOnly(async (d: string) => {
    try {
      return await loadDay(DATA, airport, d);
    } catch {
      return null;
    }
  });
  const topbar = mountTopbar(document.getElementById('top')!, index, date, async (d) => {
    date = d;
    topbar.setDay(d);
    const next = await loadLatest(d);
    if (next === undefined) return; // a newer pick superseded this one
    if (next === null) {
      day = tracks = null;
      select(null);
      timeline.setDay(empty);
      showNotice('This day could not be loaded.');
      return;
    }
    ({ day, tracks } = next);
    showNotice(null);
    select(null);
    timeline.setDay(day);
  });

  let lastUrl = '';
  let lastUrlAt = 0;
  const syncUrl = (now: number) => {
    if (now - lastUrlAt < URL_EVERY_MS) return;
    lastUrlAt = now;
    const next = writeUrl({ d: date, t: clock.t, f: selected !== null && day ? day.flights[selected].id : null });
    if (next !== lastUrl) {
      lastUrl = next;
      const keep = new URLSearchParams(location.search).has('perf') ? '&perf' : '';
      history.replaceState(null, '', next + keep);
    }
  };

  const perf = new URLSearchParams(location.search).has('perf');
  // Exposed for the performance check only.
  if (perf) (window as unknown as { __clock: Clock }).__clock = clock;
  let frames = 0;
  let spent = 0;
  let since = performance.now();
  let firstDrawn = false;

  const draw = () => {
    timeline.update(clock.t);
    if (!day || !tracks) {
      overlay.setProps({ layers: [] });
      return;
    }
    const state = stateAt(day, tracks, clock.t);
    if (perf && !firstDrawn && state.aircraft.length) {
      firstDrawn = true;
      console.log(`[perf] first aircraft at ${Math.round(performance.now())} ms after navigation`);
    }
    overlay.setProps({
      layers: buildLayers({ day, tracks, state, tMs: clock.t, selected, onPick: select }),
      onClick: (info) => {
        if (!info.object) select(null);
      },
    });
    panel.update(state, day, clock.t);
  };

  const loop = (now: number) => {
    const t0 = performance.now();
    clock.frame(now);
    draw();
    syncUrl(now);
    if (perf) {
      frames++;
      spent += performance.now() - t0;
      if (now - since > 5000) {
        console.log(`[perf] ${frames} frames, mean ${(spent / frames).toFixed(2)} ms work, ${((now - since) / frames).toFixed(1)} ms/frame`);
        frames = 0;
        spent = 0;
        since = now;
      }
    }
    requestAnimationFrame(loop);
  };
  clock.play();
  requestAnimationFrame(loop);
}

main();
