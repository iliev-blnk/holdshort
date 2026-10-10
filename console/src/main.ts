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
import { mountTimeline } from './ui/timeline';
import { mountTopbar } from './ui/topbar';
import { readUrl } from './ui/url';

const DATA = import.meta.env.VITE_DATA_BASE ?? 'data';
const BASEMAP = import.meta.env.VITE_BASEMAP ?? 'basemap/istanbul.pmtiles';
const DEFAULT_DAY = '2026-10-02';
/** 10:00 UTC: the busiest hour in coverage (about 32 aircraft in view). */
const DEFAULT_START_MS = 36_000_000;

async function main() {
  const app = document.querySelector<HTMLDivElement>('#app')!;
  app.innerHTML = `
    <header id="top"></header>
    <div id="map"></div>
    <aside id="panel"></aside>
    <footer id="timeline"></footer>`;
  const index = await loadIndex(DATA);
  const days = index.days.map((d) => d.date);
  const url = readUrl(location.search, days);
  let date = url.d ?? (days.includes(DEFAULT_DAY) ? DEFAULT_DAY : days[0]);
  let { day, tracks }: { day: DayJson; tracks: Tracks } = await loadDay(DATA, index.airport, date);

  const map = await createMap(document.getElementById('map')!, BASEMAP);
  const overlay = new MapboxOverlay({ interleaved: false, layers: [] });
  map.addControl(overlay);

  let selected: number | null = null;
  const clock = new Clock(() => {}, url.t ?? DEFAULT_START_MS, 60);
  const timeline = mountTimeline(document.getElementById('timeline')!, clock, day);
  const loadLatest = latestOnly(loadDay);
  const topbar = mountTopbar(document.getElementById('top')!, index, date, async (d) => {
    const next = await loadLatest(DATA, index.airport, d);
    if (!next) return;
    ({ day, tracks } = next);
    date = d;
    selected = null;
    topbar.setDay(d);
    timeline.setDay(day);
  });

  const perf = new URLSearchParams(location.search).has('perf');
  // Exposed for the performance check only.
  if (perf) (window as unknown as { __clock: Clock }).__clock = clock;
  let frames = 0;
  let spent = 0;
  let since = performance.now();
  let firstDrawn = false;

  const draw = () => {
    const state = stateAt(day, tracks, clock.t);
    if (perf && !firstDrawn && state.aircraft.length) {
      firstDrawn = true;
      console.log(`[perf] first aircraft at ${Math.round(performance.now())} ms after navigation`);
    }
    overlay.setProps({
      layers: buildLayers({ day, tracks, state, tMs: clock.t, selected, onPick: (f) => (selected = f) }),
      onClick: (info) => {
        if (!info.object) selected = null;
      },
    });
    timeline.update(clock.t);
  };

  const loop = (now: number) => {
    const t0 = performance.now();
    clock.frame(now);
    draw();
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
