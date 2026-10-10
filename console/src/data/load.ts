import { decodeTracks } from './decode';
import type { DayIndex, DayJson, Tracks } from './types';

async function get(url: string): Promise<Response> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r;
}

/**
 * tracks.bin is shipped gzipped because static hosts don't compress octet-stream.
 * Some servers send it with Content-Encoding: gzip, and the browser has then
 * already unpacked it, so unpack only when the gzip magic bytes are there.
 */
async function gunzipIfNeeded(buf: ArrayBuffer): Promise<ArrayBuffer> {
  const b = new Uint8Array(buf, 0, Math.min(2, buf.byteLength));
  if (b[0] !== 0x1f || b[1] !== 0x8b) return buf;
  return new Response(new Blob([buf]).stream().pipeThrough(new DecompressionStream('gzip'))).arrayBuffer();
}

export async function loadIndex(base: string): Promise<DayIndex> {
  return (await get(`${base}/index.json`)).json();
}

export async function loadDay(base: string, airport: string, date: string): Promise<{ day: DayJson; tracks: Tracks }> {
  const dir = `${base}/${airport}/${date}`;
  const [day, bin] = await Promise.all([
    get(`${dir}/day.json`).then((r) => r.json() as Promise<DayJson>),
    get(`${dir}/tracks.bin.gz`).then((r) => r.arrayBuffer()).then(gunzipIfNeeded),
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
