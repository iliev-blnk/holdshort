import type { Layer, PickingInfo } from '@deck.gl/core';
import { IconLayer, LineLayer, PathLayer, ScatterplotLayer } from '@deck.gl/layers';
import type { DayJson, EventJ, Tracks } from '../data/types';
import { RUNWAYS, ring } from '../geo/ltfm';
import { trailAt } from '../sim/interpolate';
import type { SimState } from '../sim/state';
import { T, kindColor, rgb } from '../theme';

type Aircraft = SimState['aircraft'][number];

const ORDER: Record<string, number> = { other: 0, overflight: 1, departure: 2, both: 3, arrival: 3 };
const PULSE_MS = 60_000;
const PULSE_KINDS = new Set(['touchdown', 'liftoff', 'go_around', 'hold_start']);
const RING = ring(50);

/** A white arrow pointing north, used as a mask so each aircraft takes its kind's colour. */
function arrowAtlas(): string {
  const c = document.createElement('canvas');
  c.width = c.height = 64;
  const g = c.getContext('2d')!;
  g.fillStyle = '#fff';
  g.beginPath();
  g.moveTo(32, 4);
  g.lineTo(54, 58);
  g.lineTo(32, 46);
  g.lineTo(10, 58);
  g.closePath();
  g.fill();
  return c.toDataURL();
}

let atlas: string | null = null;
const fullPaths = new WeakMap<Tracks, Map<number, [number, number][]>>();

function fullPath(tr: Tracks, f: number, p: [number, number]): [number, number][] {
  let m = fullPaths.get(tr);
  if (!m) fullPaths.set(tr, (m = new Map()));
  let path = m.get(f);
  if (!path) {
    path = [];
    for (let i = p[0]; i < p[0] + p[1]; i++) path.push([tr.lon[i] / 1e5, tr.lat[i] / 1e5]);
    m.set(f, path);
  }
  return path;
}

export interface LayerInput {
  day: DayJson;
  tracks: Tracks;
  state: SimState;
  tMs: number;
  selected: number | null;
  onPick: (f: number | null) => void;
}

export function buildLayers({ day, tracks, state, tMs, selected, onPick }: LayerInput): Layer[] {
  atlas ??= arrowAtlas();
  const aircraft = [...state.aircraft].sort((a, b) => ORDER[a.kind] - ORDER[b.kind]);
  const trails = aircraft.map((a) => ({ a, path: trailAt(tracks, day.flights[a.f].p, tMs) }));

  const pulses: { e: EventJ; age: number }[] = [];
  for (const i of state.events) {
    const e = day.events[i];
    if (e.t <= tMs - PULSE_MS) break;
    if (PULSE_KINDS.has(e.kind) && e.lat !== null && e.lon !== null) pulses.push({ e, age: (tMs - e.t) / PULSE_MS });
  }
  const pulseColor = (kind: string) => rgb(kind === 'liftoff' ? T.dep : T.arr);

  const out: Layer[] = [
    new PathLayer({
      id: 'ring',
      data: [RING],
      getPath: (d: [number, number][]) => d,
      getColor: rgb(T.line),
      widthUnits: 'pixels',
      getWidth: 1,
    }),
    new LineLayer({
      id: 'runways',
      data: RUNWAYS,
      getSourcePosition: (d: (typeof RUNWAYS)[number]) => d[0],
      getTargetPosition: (d: (typeof RUNWAYS)[number]) => d[1],
      getColor: rgb(T.text),
      getWidth: 3,
    }),
    new PathLayer({
      id: 'trails',
      data: trails,
      getPath: (d: (typeof trails)[number]) => d.path,
      getColor: (d: (typeof trails)[number]) => [...kindColor(d.a.kind), 120],
      widthUnits: 'pixels',
      getWidth: 1.5,
      updateTriggers: { getPath: tMs },
    }),
    new ScatterplotLayer({
      id: 'pulses',
      data: pulses,
      getPosition: (d: (typeof pulses)[number]) => [d.e.lon!, d.e.lat!],
      getRadius: (d: (typeof pulses)[number]) => 300 + d.age * 2500,
      getLineColor: (d: (typeof pulses)[number]) => [...pulseColor(d.e.kind), Math.round(255 * (1 - d.age))],
      stroked: true,
      filled: false,
      lineWidthUnits: 'pixels',
      getLineWidth: 1.5,
      updateTriggers: { getRadius: tMs, getLineColor: tMs },
    }),
  ];
  if (selected !== null) {
    out.push(
      new PathLayer({
        id: 'selected-path',
        data: [fullPath(tracks, selected, day.flights[selected].p)],
        getPath: (d: [number, number][]) => d,
        getColor: rgb(T.arr),
        widthUnits: 'pixels',
        getWidth: 2,
      }),
    );
  }
  out.push(
    new IconLayer({
      id: 'aircraft',
      data: aircraft,
      iconAtlas: atlas,
      iconMapping: { arrow: { x: 0, y: 0, width: 64, height: 64, mask: true } },
      getIcon: () => 'arrow',
      getPosition: (d: Aircraft) => [d.pos.lon, d.pos.lat],
      getAngle: (d: Aircraft) => -d.pos.trk,
      getSize: (d: Aircraft) => (d.kind === 'other' || d.kind === 'overflight' ? 10 : 14) + (d.f === selected ? 6 : 0),
      sizeUnits: 'pixels',
      getColor: (d: Aircraft) => (d.f === selected ? [255, 255, 255] : kindColor(d.kind)),
      pickable: true,
      onClick: (info: PickingInfo<Aircraft>) => {
        if (info.object) onPick(info.object.f);
        return true;
      },
      updateTriggers: { getPosition: tMs, getAngle: tMs, getSize: selected, getColor: selected },
    }),
  );
  return out;
}
