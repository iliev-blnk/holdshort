"""Figures for the technical write-up (docs/WRITEUP.md → docs/writeup-figures/).

The hero picture draws every position of one day within 50 nm as light: each kind of flight
accumulates into its own density grid, and the grids are added in their console colours.
"""

import shutil
from pathlib import Path

import numpy as np
import polars as pl
from PIL import Image

LTFM = (41.2753, 28.7519)  # the reference point in crates/core/src/airport.rs
RADIUS_NM = 50.0
SIZE = (2400, 1350)
MARGIN = 1.04
MAX_GAP_MS = 600_000  # the coverage rule: straight legs are thinned in the history, not lost
STEP_PX = 0.5
# Densities at this percentile map to 63% brightness: corridors glow, single tracks stay visible.
TONE_PCT = 90
BG = np.array([0x1B, 0x1F, 0x24]) / 255
# kind → (colour, weight)
KINDS = {
    "arrival": (np.array([0x5A, 0xA9, 0xFF]) / 255, 1.0),
    "departure": (np.array([0xFF, 0x9F, 0x43]) / 255, 1.0),
    "other": (np.array([0x8D, 0x97, 0xA1]) / 255, 0.35),
}
# Copied as they are: (source relative to the repo, name in docs/writeup-figures/).
COPIES = [
    ("research/figures/eta_scatter.png", "eta_scatter.png"),
    ("research/figures/hourly_test_week.png", "hourly_test_week.png"),
    ("research/figures/anomaly_scores.png", "anomaly_scores.png"),
    ("research/figures/anomaly_gallery_1.png", "anomaly_gallery_1.png"),
    ("docs/console.png", "console.png"),
]


def flight_kinds(obs: pl.DataFrame, flights: pl.DataFrame) -> pl.DataFrame:
    """obs with a `kind` column: arrival (or both), departure, else other, by hex and time window."""
    f = flights.select("hex", "first_ms", "last_ms", kind=pl.col("kind").replace({"both": "arrival"}))
    o = obs.with_row_index("_i")
    hit = (
        o.join(f, on="hex")
        .filter(pl.col("ts_ms").is_between("first_ms", "last_ms"))
        .filter(pl.col("kind").is_in(["arrival", "departure"]))
        .group_by("_i")
        .agg(pl.col("kind").first())
    )
    return (
        o.join(hit, on="_i", how="left")
        .with_columns(pl.col("kind").fill_null("other"))
        .sort("_i")
        .drop("_i")
    )


def to_px(lat: np.ndarray, lon: np.ndarray, center: tuple[float, float], radius_nm: float, size: tuple[int, int]):
    w, h = size
    scale = h / (2 * radius_nm * MARGIN)  # px per nm
    x = w / 2 + (lon - center[1]) * 60 * np.cos(np.radians(center[0])) * scale
    y = h / 2 - (lat - center[0]) * 60 * scale
    return x, y


def _density(seg: pl.DataFrame, center, radius_nm, size) -> np.ndarray:
    w, h = size
    grid = np.zeros((h, w))
    if seg.height < 2:
        return grid
    seg = seg.sort("hex", "ts_ms")
    x, y = to_px(seg["lat"].to_numpy(), seg["lon"].to_numpy(), center, radius_nm, size)
    t = seg["ts_ms"].to_numpy()
    hx = seg["hex"].to_numpy()
    ok = (hx[1:] == hx[:-1]) & (np.diff(t) <= MAX_GAP_MS)
    x0, y0, x1, y1 = x[:-1][ok], y[:-1][ok], x[1:][ok], y[1:][ok]
    n = np.maximum(1, np.ceil(np.hypot(x1 - x0, y1 - y0) / STEP_PX)).astype(int)
    f = np.concatenate([np.arange(k) / k for k in n]) if len(n) else np.array([])
    xs = np.repeat(x0, n) + f * np.repeat(x1 - x0, n)
    ys = np.repeat(y0, n) + f * np.repeat(y1 - y0, n)
    xi, yi = np.floor(xs).astype(int), np.floor(ys).astype(int)
    keep = (xi >= 0) & (xi < w) & (yi >= 0) & (yi < h)
    np.add.at(grid, (yi[keep], xi[keep]), 1.0)
    return grid


def render_day(obs: pl.DataFrame, center: tuple[float, float], radius_nm: float, size: tuple[int, int]) -> np.ndarray:
    """(h, w, 3) image in [0, 1]: each kind's density tone-mapped and added in its colour."""
    img = np.broadcast_to(BG, (size[1], size[0], 3)).copy()
    for kind, (colour, weight) in KINDS.items():
        d = _density(obs.filter(pl.col("kind") == kind), center, radius_nm, size)
        nz = d[d > 0]
        if nz.size == 0:
            continue
        s = np.percentile(nz, TONE_PCT)
        img += (1 - np.exp(-d / s))[..., None] * colour * weight
    return np.clip(img, 0, 1)


def run(data_dir: Path, repo: Path, date: str = "2026-10-02") -> list[Path]:
    out = Path(repo) / "docs" / "writeup-figures"
    out.mkdir(parents=True, exist_ok=True)
    obs = pl.read_parquet(Path(data_dir, "LTFM", "parquet", f"date={date}", "obs.parquet"), columns=["ts_ms", "hex", "lat", "lon"])
    flights = pl.read_parquet(Path(data_dir, "LTFM", "twin", f"date={date}", "flights.parquet"), columns=["hex", "kind", "first_ms", "last_ms"])
    img = render_day(flight_kinds(obs, flights), LTFM, RADIUS_NM, SIZE)
    # WebP keeps the hero light (a PNG of this many thin lines is ~1.7 MB).
    day = out / "day.webp"
    Image.fromarray((img * 255).round().astype(np.uint8)).save(day, "WEBP", quality=85, method=6)
    paths = [day]
    for src, name in COPIES:
        shutil.copyfile(Path(repo) / src, out / name)
        paths.append(out / name)
    return paths
