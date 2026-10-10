"""Baselines the landing-time model has to beat. All use only what is known at ring entry."""

from dataclasses import dataclass

import numpy as np
import polars as pl

KT_NM_PER_S = 1 / 3600


@dataclass
class EtaBaselines:
    median: float
    groups: dict  # (sector, arr_rwys) -> median seconds
    ratio: float  # actual / straight-line estimate, median over training rows

    @classmethod
    def fit(cls, train: pl.DataFrame, min_group: int = 20) -> "EtaBaselines":
        median = float(train["asma_s"].median())
        g = (
            train.group_by("sector", "arr_rwys")
            .agg(pl.col("asma_s").median().alias("m"), pl.len().alias("n"))
            .filter(pl.col("n") >= min_group)
        )
        groups = {(int(s), float(r)): float(m) for s, r, m in g.select("sector", "arr_rwys", "m").iter_rows()}
        est = _straight_line_s(train)
        ok = np.isfinite(est) & (est > 0)
        ratio = float(np.median(train["asma_s"].to_numpy()[ok] / est[ok])) if ok.any() else 1.0
        return cls(median, groups, ratio)

    def global_median(self, df: pl.DataFrame) -> np.ndarray:
        """Baseline 1: the training median for everyone."""
        return np.full(len(df), self.median)

    def sector_runway(self, df: pl.DataFrame) -> np.ndarray:
        """Baseline 2: median by entry sector x number of arrival runways in use."""
        return np.array(
            [self.groups.get((int(s), float(r)), self.median) for s, r in df.select("sector", "arr_rwys").iter_rows()]
        )

    def distance_speed(self, df: pl.DataFrame) -> np.ndarray:
        """Baseline 3: straight-line distance / ground speed, scaled; baseline 2 without a speed."""
        est = _straight_line_s(df) * self.ratio
        return np.where(np.isfinite(est), est, self.sector_runway(df))


def _straight_line_s(df: pl.DataFrame) -> np.ndarray:
    gs = df["gs_kt"].to_numpy().astype(float)
    with np.errstate(divide="ignore", invalid="ignore"):
        return df["dist_nm"].to_numpy() / (gs * KT_NM_PER_S)
