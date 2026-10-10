// Unpack a .pmtiles extract into plain {z}/{x}/{y}.pbf files plus tiles.json.
// Static hosts like Cloudflare Pages ignore HTTP range requests on large files,
// which PMTiles needs; plain tile files work everywhere.
// Usage: node scripts/basemap-tiles.mjs <in.pmtiles> <out dir>
import { mkdirSync, openSync, readSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { PMTiles } from 'pmtiles';

const [input, out] = process.argv.slice(2);
if (!input || !out) {
  console.error('usage: node scripts/basemap-tiles.mjs <in.pmtiles> <out dir>');
  process.exit(2);
}

const fd = openSync(input, 'r');
const source = {
  getKey: () => input,
  async getBytes(offset, length) {
    const buf = Buffer.alloc(length);
    const n = readSync(fd, buf, 0, length, offset);
    return { data: buf.buffer.slice(buf.byteOffset, buf.byteOffset + n) };
  },
};
const pm = new PMTiles(source);
const h = await pm.getHeader();

const lon2x = (lon, z) => Math.floor(((lon + 180) / 360) * 2 ** z);
const lat2y = (lat, z) => {
  const r = (lat * Math.PI) / 180;
  return Math.floor(((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2) * 2 ** z);
};

rmSync(join(out, 'tiles'), { recursive: true, force: true });
let count = 0;
let bytes = 0;
for (let z = h.minZoom; z <= h.maxZoom; z++) {
  for (let x = lon2x(h.minLon, z); x <= lon2x(h.maxLon, z); x++) {
    for (let y = lat2y(h.maxLat, z); y <= lat2y(h.minLat, z); y++) {
      const tile = await pm.getZxy(z, x, y);
      if (!tile) continue;
      const dir = join(out, 'tiles', String(z), String(x));
      mkdirSync(dir, { recursive: true });
      writeFileSync(join(dir, `${y}.pbf`), Buffer.from(tile.data));
      count++;
      bytes += tile.data.byteLength;
    }
  }
}
const tilejson = {
  minzoom: h.minZoom,
  maxzoom: h.maxZoom,
  bounds: [h.minLon, h.minLat, h.maxLon, h.maxLat],
};
writeFileSync(join(out, 'tiles.json'), JSON.stringify(tilejson));
console.log(`${count} tiles, ${(bytes / 1024 / 1024).toFixed(1)} MB unpacked → ${out}/tiles`);
