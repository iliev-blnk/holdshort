/** Istanbul Airport: reference point and runway thresholds (from crates/core/src/airport.rs). */
export const LTFM = { lon: 28.7519, lat: 41.2753 };

/** Each runway as its two thresholds, [lon, lat]. */
export const RUNWAYS: [[number, number], [number, number]][] = [
  [[28.766272, 41.253693], [28.800018, 41.254063]], // 09/27
  [[28.7092, 41.298599], [28.7099, 41.2649]], // 16L/34R
  [[28.706751, 41.298599], [28.707411, 41.264839]], // 16R/34L
  [[28.727039, 41.298828], [28.72776, 41.261921]], // 17L/35R
  [[28.724501, 41.298801], [28.7253, 41.261902]], // 17R/35L
  [[28.756201, 41.289799], [28.756701, 41.262199]], // 18/36
];

/** A circle of `nm` nautical miles around LTFM as a closed [lon, lat] path. */
export function ring(nm: number, steps = 180): [number, number][] {
  const dLat = nm / 60;
  const dLon = dLat / Math.cos((LTFM.lat * Math.PI) / 180);
  return Array.from({ length: steps + 1 }, (_, i) => {
    const a = (i / steps) * 2 * Math.PI;
    return [LTFM.lon + dLon * Math.sin(a), LTFM.lat + dLat * Math.cos(a)];
  });
}
