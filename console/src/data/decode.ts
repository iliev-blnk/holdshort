import type { Tracks } from './types';

const HEADER = 24;
const NO_ALT = -32768;
const NO_U16 = 65535;

/** Views over a tracks.bin buffer; nothing is copied. */
export function decodeTracks(buf: ArrayBuffer): Tracks {
  if (buf.byteLength < HEADER) throw new Error('tracks.bin too short');
  const v = new DataView(buf);
  const magic = String.fromCharCode(v.getUint8(0), v.getUint8(1), v.getUint8(2), v.getUint8(3));
  if (magic !== 'HSTK') throw new Error('bad magic');
  const version = v.getUint32(4, true);
  if (version !== 1) throw new Error(`unsupported version ${version}`);
  const n = v.getUint32(8, true);
  if (buf.byteLength !== HEADER + 18 * n) throw new Error('tracks.bin size does not match n');
  return {
    n,
    dayStartMs: Number(v.getBigInt64(16, true)),
    t: new Int32Array(buf, HEADER, n),
    lat: new Int32Array(buf, HEADER + 4 * n, n),
    lon: new Int32Array(buf, HEADER + 8 * n, n),
    alt: new Int16Array(buf, HEADER + 12 * n, n),
    trk: new Uint16Array(buf, HEADER + 14 * n, n),
    gs: new Uint16Array(buf, HEADER + 16 * n, n),
  };
}

export const altAt = (tr: Tracks, i: number): number | null => (tr.alt[i] === NO_ALT ? null : tr.alt[i]);
/** Track in degrees. */
export const trkAt = (tr: Tracks, i: number): number | null => (tr.trk[i] === NO_U16 ? null : tr.trk[i] / 100);
/** Ground speed in knots. */
export const gsAt = (tr: Tracks, i: number): number | null => (tr.gs[i] === NO_U16 ? null : tr.gs[i] / 10);
export const lonLatAt = (tr: Tracks, i: number): [number, number] => [tr.lon[i] / 1e5, tr.lat[i] / 1e5];
