import type { DayJson } from '../data/types';
import { type Clock, DAY_MS, type Speed } from '../sim/clock';
import { T } from '../theme';

export interface Pad {
  l: number;
  r: number;
}

/** Canvas x → ms since day start, clamped to the day. */
export function xToMs(x: number, width: number, pad: Pad): number {
  const f = (x - pad.l) / (width - pad.l - pad.r);
  return Math.min(Math.max(f, 0), 1) * DAY_MS;
}

export function msToX(tMs: number, width: number, pad: Pad): number {
  return pad.l + (tMs / DAY_MS) * (width - pad.l - pad.r);
}

const PAD: Pad = { l: 12, r: 18 };
const SPEEDS: Speed[] = [30, 60, 300, 600];
const RIBBON_Y = 18;
const MID = 64;
const BAR_H = 30;
const MIN_LABEL_PX = 90;

export interface Timeline {
  update(tMs: number): void;
  setDay(day: DayJson): void;
}

export function mountTimeline(el: HTMLElement, clock: Clock, first: DayJson): Timeline {
  el.innerHTML = `
    <div class="tl-controls">
      <button class="tl-play" aria-label="Play">▶</button>
      <div class="tl-speeds">${SPEEDS.map((s) => `<button data-speed="${s}">${s}×</button>`).join('')}</div>
    </div>
    <canvas class="tl-canvas" aria-label="Timeline: drag to move through the day"></canvas>`;
  const play = el.querySelector<HTMLButtonElement>('.tl-play')!;
  const canvas = el.querySelector<HTMLCanvasElement>('.tl-canvas')!;
  const g = canvas.getContext('2d')!;
  const staticLayer = document.createElement('canvas');
  let day = first;
  let w = 0;
  let h = 0;
  let lastT = -1;
  let lastPlaying: boolean | null = null;

  const speedButtons = [...el.querySelectorAll<HTMLButtonElement>('[data-speed]')];
  const markSpeed = () =>
    speedButtons.forEach((b) => b.classList.toggle('on', Number(b.dataset.speed) === clock.speed));
  speedButtons.forEach((b) =>
    b.addEventListener('click', () => {
      clock.setSpeed(Number(b.dataset.speed) as Speed);
      markSpeed();
    }),
  );
  markSpeed();
  play.addEventListener('click', () => clock.toggle());

  function drawStatic() {
    const dpr = window.devicePixelRatio || 1;
    w = canvas.clientWidth;
    h = canvas.clientHeight;
    for (const c of [canvas, staticLayer]) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
    }
    const s = staticLayer.getContext('2d')!;
    s.setTransform(dpr, 0, 0, dpr, 0, 0);
    s.clearRect(0, 0, w, h);
    const x = (t: number) => msToX(t, w, PAD);

    // Configuration ribbon, alternating shades, labelled where there is room.
    day.config.forEach((c, i) => {
      const x0 = x(c.t);
      const x1 = x(day.config[i + 1]?.t ?? DAY_MS);
      s.fillStyle = i % 2 ? T.muted : T.line;
      s.globalAlpha = 0.7;
      s.fillRect(x0, RIBBON_Y, Math.max(x1 - x0 - 1, 1), 6);
      s.globalAlpha = 1;
      if (x1 - x0 >= MIN_LABEL_PX) {
        s.fillStyle = T.muted;
        s.font = '10px "JetBrains Mono", monospace';
        s.fillText(c.label || '—', x0 + 2, RIBBON_Y - 4, x1 - x0 - 4);
      }
    });

    // Movements: arrivals above the line, departures below.
    const { arr, dep } = day.hist;
    const peak = Math.max(1, ...arr, ...dep);
    const bw = (w - PAD.l - PAD.r) / arr.length;
    for (let i = 0; i < arr.length; i++) {
      const bx = PAD.l + i * bw;
      s.fillStyle = T.arr;
      const ha = (arr[i] / peak) * BAR_H;
      s.fillRect(bx, MID - ha, Math.max(bw - 1, 1), ha);
      s.fillStyle = T.dep;
      s.fillRect(bx, MID + 1, Math.max(bw - 1, 1), (dep[i] / peak) * BAR_H);
    }

    // Ticks for go-arounds and configuration changes.
    for (const e of day.events) {
      if (e.kind !== 'go_around' && e.kind !== 'config_change') continue;
      s.fillStyle = e.kind === 'go_around' ? T.text : T.muted;
      s.fillRect(Math.round(x(e.t)), RIBBON_Y + 8, 1, e.kind === 'go_around' ? 8 : 4);
    }

    s.fillStyle = T.muted;
    s.font = '10px "JetBrains Mono", monospace';
    s.textAlign = 'center';
    for (let hr = 0; hr <= 24; hr += 3) s.fillText(String(hr).padStart(2, '0'), x(hr * 3_600_000), h - 6);
    s.textAlign = 'start';
    lastT = -1;
  }

  function update(tMs: number) {
    if (lastPlaying !== clock.playing) {
      lastPlaying = clock.playing;
      play.textContent = clock.playing ? '❚❚' : '▶';
      play.setAttribute('aria-label', clock.playing ? 'Pause' : 'Play');
    }
    if (tMs === lastT) return;
    lastT = tMs;
    const dpr = window.devicePixelRatio || 1;
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.clearRect(0, 0, canvas.width, canvas.height);
    g.drawImage(staticLayer, 0, 0);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const px = Math.round(msToX(tMs, w, PAD)) + 0.5;
    g.strokeStyle = T.arr;
    g.lineWidth = 1.5;
    g.beginPath();
    g.moveTo(px, 8);
    g.lineTo(px, h - 18);
    g.stroke();
  }

  // Scrubbing with mouse, pen or touch.
  let dragging = false;
  const seekTo = (e: PointerEvent) => {
    const r = canvas.getBoundingClientRect();
    clock.seek(xToMs(e.clientX - r.left, r.width, PAD));
  };
  canvas.addEventListener('pointerdown', (e) => {
    dragging = true;
    canvas.setPointerCapture(e.pointerId);
    seekTo(e);
  });
  canvas.addEventListener('pointermove', (e) => dragging && seekTo(e));
  canvas.addEventListener('pointerup', () => (dragging = false));
  canvas.addEventListener('pointercancel', () => (dragging = false));

  window.addEventListener('keydown', (e) => {
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === 'SELECT' || tag === 'INPUT' || tag === 'TEXTAREA') return;
    if (e.code === 'Space') {
      e.preventDefault();
      clock.toggle();
    } else if (e.key === 'ArrowLeft') clock.seek(clock.t - 60_000);
    else if (e.key === 'ArrowRight') clock.seek(clock.t + 60_000);
  });

  new ResizeObserver(() => {
    drawStatic();
    update(clock.t);
  }).observe(canvas);
  drawStatic();
  document.fonts?.ready.then(() => {
    drawStatic();
    update(clock.t);
  });

  return {
    update,
    setDay(d: DayJson) {
      day = d;
      drawStatic();
      update(clock.t);
    },
  };
}
