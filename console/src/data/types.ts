/** Shapes of the files `holdshort export` writes. Times are ms since the day start. */

export type Kind = 'arrival' | 'departure' | 'both' | 'overflight' | 'other';

export interface DaySummary {
  date: string;
  arrivals: number;
  departures: number;
  go_arounds: number;
  asma_median_s: number | null;
}

export interface DayIndex {
  v: 1;
  airport: string;
  days: DaySummary[];
}

export interface FlightJ {
  id: string;
  cs: string | null;
  type: string | null;
  reg: string | null;
  kind: Kind;
  arr_rwy: string | null;
  td: number | null;
  dep_rwy: string | null;
  lo: number | null;
  /** The touchdown or liftoff time is an estimate. */
  low: boolean;
  asma_s: number | null;
  ga: number;
  holds: number;
  hold_s: number;
  /** First point and point count in tracks.bin. */
  p: [number, number];
  /** Landing time predicted at the 40 nm ring, seconds after entry (null without a prediction). */
  eta_s: number | null;
  /** Approach anomaly score (arrivals exported with models). */
  ap: ApJ | null;
  /** The nearest normal approach as [lon, lat] points (flagged approaches only). */
  ap_ref: [number, number][] | null;
}

/** A metric far from normal: its value and the typical value. */
export interface ReasonJ {
  m: string;
  v: number;
  typ: number;
}

export interface ApJ {
  /** Percentile among normal approaches to the same runway; null when not scored. */
  pct: number | null;
  /** In the top 1%. */
  flag: boolean;
  reasons: ReasonJ[];
  /** Why there is no score: lost_on_final, no_entry, runway or coverage. */
  unscored: string | null;
}

export interface EventJ {
  t: number;
  kind: 'asma_entry' | 'go_around' | 'hold_start' | 'hold_end' | 'touchdown' | 'liftoff' | 'config_change' | 'unusual_approach';
  /** Index into flights; null for configuration changes. */
  f: number | null;
  rwy: string | null;
  lat: number | null;
  lon: number | null;
  detail: string | null;
}

export interface ConfigJ {
  t: number;
  label: string;
}

export interface DayJson {
  v: 1;
  airport: string;
  date: string;
  day_start_ms: number;
  flights: FlightJ[];
  events: EventJ[];
  config: ConfigJ[];
  hist: { bin_s: number; arr: number[]; dep: number[] };
}

/** tracks.bin as typed-array columns. t is in 0.1 s since the day start. */
export interface Tracks {
  n: number;
  dayStartMs: number;
  t: Int32Array;
  lat: Int32Array;
  lon: Int32Array;
  alt: Int16Array;
  trk: Uint16Array;
  gs: Uint16Array;
}
