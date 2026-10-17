import type { Kind } from './data/types';

/** Slate: the console's palette (mirrors the custom properties in style.css). */
export const T = {
  water: '#1b1f24',
  land: '#262b31',
  coast: '#343b43',
  ui: '#20252b',
  line: '#323940',
  text: '#e6e9ec',
  muted: '#8d97a1',
  arr: '#5aa9ff',
  dep: '#ff9f43',
  ovf: '#6b7580',
  oth: '#4a535c',
  anom: '#c792ea',
} as const;

export type Rgb = [number, number, number];

export function rgb(hex: string): Rgb {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

const KIND: Record<Kind, Rgb> = {
  arrival: rgb(T.arr),
  both: rgb(T.arr),
  departure: rgb(T.dep),
  overflight: rgb(T.ovf),
  other: rgb(T.oth),
};

export const kindColor = (k: Kind): Rgb => KIND[k];
