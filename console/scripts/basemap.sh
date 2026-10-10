#!/usr/bin/env bash
# Extract the Istanbul basemap from a Protomaps daily planet build.
# Usage: scripts/basemap.sh [YYYYMMDD]   (default: yesterday, UTC)
set -euo pipefail
cd "$(dirname "$0")/.."

date=${1:-$(date -u -d yesterday +%Y%m%d)}
out=public/basemap/istanbul.pmtiles
max_bytes=$((20 * 1024 * 1024))

if ! command -v pmtiles >/dev/null; then
  v=1.31.2
  echo "installing go-pmtiles $v to ~/.local/bin"
  mkdir -p ~/.local/bin
  curl -fsSL "https://github.com/protomaps/go-pmtiles/releases/download/v$v/go-pmtiles_${v}_Linux_x86_64.tar.gz" |
    tar -xz -C ~/.local/bin pmtiles
fi

mkdir -p "$(dirname "$out")"
pmtiles extract "https://build.protomaps.com/$date.pmtiles" "$out" --bbox=27.6,40.5,29.9,42.0 --maxzoom=12
size=$(stat -c %s "$out")
echo "$out: $((size / 1024)) KB"
if [ "$size" -gt "$max_bytes" ]; then
  echo "basemap is over 20 MB" >&2
  exit 1
fi
