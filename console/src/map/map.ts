import { layers, namedFlavor, type Flavor } from '@protomaps/basemaps';
import { Map as MapLibre, type StyleSpecification, setWorkerUrl } from 'maplibre-gl';
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?url';
import 'maplibre-gl/dist/maplibre-gl.css';
import { LTFM } from '../geo/ltfm';
import { T } from '../theme';

export const ATTRIBUTION = '© OpenStreetMap contributors · Protomaps';

// MapLibre 6 loads its worker as a separate module; let Vite bundle and point at it.
setWorkerUrl(workerUrl);

/** The dark Protomaps flavor recoloured to Slate: flat land, quiet roads, no labels. */
function slate(): Flavor {
  const base = namedFlavor('dark');
  const f: Record<string, unknown> = { ...base };
  for (const [k, v] of Object.entries(base)) {
    if (typeof v !== 'string' || !v.startsWith('#')) continue;
    if (k === 'water') f[k] = T.water;
    else if (k === 'boundaries' || k === 'runway') f[k] = T.coast;
    else if (/highway|major|railway/.test(k)) f[k] = '#2f353c';
    else if (/minor|link|other|tunnel|bridges|pier|service/.test(k)) f[k] = '#2a2f35';
    else if (k === 'aerodrome') f[k] = '#2b3138';
    else if (!/label|halo|regular|bold|italic/.test(k)) f[k] = T.land;
  }
  if (base.landcover) {
    f.landcover = Object.fromEntries(Object.keys(base.landcover).map((k) => [k, T.land]));
  }
  return f as unknown as Flavor;
}

function plainStyle(): StyleSpecification {
  return {
    version: 8,
    sources: {},
    layers: [{ id: 'background', type: 'background', paint: { 'background-color': T.water } }],
  };
}

/** Style for the unpacked tiles described by `tiles.json` (see scripts/basemap-tiles.mjs). */
async function basemapStyle(url: string): Promise<StyleSpecification> {
  if (url === 'none') return plainStyle();
  const abs = new URL(url, location.href);
  let tj: { minzoom: number; maxzoom: number; bounds: [number, number, number, number] };
  try {
    const r = await fetch(abs);
    if (!r.ok) return plainStyle();
    tj = await r.json();
  } catch {
    return plainStyle();
  }
  // MapLibre needs an absolute template; keep {z}/{x}/{y} unescaped.
  const tiles = `${new URL('tiles/', abs).href}{z}/{x}/{y}.pbf`;
  return {
    version: 8,
    sources: { protomaps: { type: 'vector', tiles: [tiles], minzoom: tj.minzoom, maxzoom: tj.maxzoom, bounds: tj.bounds, attribution: ATTRIBUTION } },
    layers: layers('protomaps', slate()),
  };
}

export async function createMap(el: HTMLElement, basemapUrl: string): Promise<MapLibre> {
  const map = new MapLibre({
    container: el,
    style: await basemapStyle(basemapUrl),
    center: [LTFM.lon, LTFM.lat],
    zoom: 8.6,
    minZoom: 6,
    maxZoom: 13,
    attributionControl: { compact: true, customAttribution: ATTRIBUTION },
    dragRotate: false,
    pitchWithRotate: false,
  });
  map.touchZoomRotate.disableRotation();
  return map;
}
