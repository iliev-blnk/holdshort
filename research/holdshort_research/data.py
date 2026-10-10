"""Load the feature tables written by `holdshort features` and split them by day."""

from pathlib import Path

import polars as pl

# Whole days, in time order. The test week is only used for the final report.
SPLIT = {
    "train": ("2026-09-01", "2026-09-24"),
    "val": ("2026-09-25", "2026-10-01"),
    "test": ("2026-10-02", "2026-10-08"),
}


def _tables(data_dir: Path, airport: str, name: str) -> pl.DataFrame:
    files = sorted(Path(data_dir, airport, "features").glob(f"date=*/{name}.parquet"))
    if not files:
        raise FileNotFoundError(f"no {name}.parquet under {data_dir}/{airport}/features (run `holdshort features`)")
    return pl.concat([pl.read_parquet(f) for f in files])


def load_eta(data_dir: Path, airport: str = "LTFM") -> pl.DataFrame:
    """One row per arrival at its 40 nm ring entry (see crates/twin/src/features.rs)."""
    return _tables(data_dir, airport, "eta")


def load_hourly(data_dir: Path, airport: str = "LTFM") -> pl.DataFrame:
    """Touchdowns per UTC hour, with `ts` = the start of the hour."""
    df = _tables(data_dir, airport, "hourly")
    return df.with_columns(
        ts=pl.col("date").str.to_datetime("%Y-%m-%d") + pl.duration(hours=pl.col("hour").cast(pl.Int64))
    ).sort("ts")


def split(df: pl.DataFrame, part: str) -> pl.DataFrame:
    lo, hi = SPLIT[part]
    return df.filter(pl.col("date").is_between(pl.lit(lo), pl.lit(hi)))
