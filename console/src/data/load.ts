import { decodeTracks } from './decode';
import type { DayIndex, DayJson, Tracks } from './types';

async function get(url: string): Promise<Response> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r;
}

export async function loadIndex(base: string): Promise<DayIndex> {
  return (await get(`${base}/index.json`)).json();
}

export async function loadDay(base: string, airport: string, date: string): Promise<{ day: DayJson; tracks: Tracks }> {
  const dir = `${base}/${airport}/${date}`;
  const [day, bin] = await Promise.all([
    get(`${dir}/day.json`).then((r) => r.json() as Promise<DayJson>),
    get(`${dir}/tracks.bin`).then((r) => r.arrayBuffer()),
  ]);
  return { day, tracks: decodeTracks(bin) };
}

/** Wrap an async function so only the latest call's result is delivered; superseded calls resolve undefined. */
export function latestOnly<A extends unknown[], R>(fn: (...a: A) => Promise<R>): (...a: A) => Promise<R | undefined> {
  let seq = 0;
  return async (...a: A) => {
    const mine = ++seq;
    const r = await fn(...a);
    return mine === seq ? r : undefined;
  };
}
