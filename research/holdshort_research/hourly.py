"""Arrivals per hour, 1–3 hours ahead.

For a target hour H and horizon h the forecast is issued at H − h + 1 (the top of an
hour), so it may only use hours that are complete by then. Missing hours (a day
without data) stay NaN; they are never filled from another day.
"""

import math
from datetime import timedelta

import numpy as np
import polars as pl

from .data import split

HOURLY_FEATURES = ["hour_sin", "hour_cos", "dow", "lag1", "lag2", "lag3", "same_yday", "same_week", "profile"]
HORIZONS = (1, 2, 3)


def profile(counts: pl.DataFrame) -> list[float]:
    """Mean arrivals per hour of day over the training days only."""
    train = split(counts, "train")
    by = dict(train.group_by("hour").agg(pl.col("arrivals").mean()).iter_rows())
    return [float(by.get(h, np.nan)) for h in range(24)]


def features(counts: pl.DataFrame, horizon: int, profile: list[float]) -> pl.DataFrame:
    """One row per target hour: the features known at issue time, plus the target `y`."""
    seen = dict(zip(counts["ts"].to_list(), counts["arrivals"].to_list()))
    hour = timedelta(hours=1)

    def at(ts):
        v = seen.get(ts)
        return float(v) if v is not None else math.nan

    rows = []
    for ts in sorted(seen):
        issue = ts - (horizon - 1) * hour
        frac = ts.hour / 24
        rows.append(
            {
                "ts": ts,
                "date": ts.strftime("%Y-%m-%d"),
                "y": float(seen[ts]),
                "hour_sin": math.sin(2 * math.pi * frac),
                "hour_cos": math.cos(2 * math.pi * frac),
                "dow": float(ts.weekday()),
                "lag1": at(issue - hour),
                "lag2": at(issue - 2 * hour),
                "lag3": at(issue - 3 * hour),
                "same_yday": at(ts - 24 * hour),
                "same_week": at(ts - 7 * 24 * hour),
                "profile": float(profile[ts.hour]),
            }
        )
    return pl.DataFrame(rows)


def baselines(df: pl.DataFrame) -> dict[str, np.ndarray]:
    """Persistence (last completed hour), same hour yesterday, hour-of-day profile.
    A missing value falls back to the profile, so every baseline predicts every row."""
    prof = df["profile"].to_numpy()

    def filled(col):
        v = df[col].to_numpy()
        return np.where(np.isnan(v), prof, v)

    return {"persistence": filled("lag1"), "same_yday": filled("same_yday"), "profile": prof}


def matrix(df: pl.DataFrame) -> np.ndarray:
    return df.select(HOURLY_FEATURES).to_numpy().astype(np.float32)
