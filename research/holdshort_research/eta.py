"""Landing time predicted when an arrival first comes within 40 nm (its ASMA time)."""

import json
from pathlib import Path

import numpy as np
import polars as pl

from . import model
from .baselines import EtaBaselines
from .metrics import summary

# Same order as ETA_FEATURES in crates/twin/src/features.rs (Rust computes the rows).
ETA_FEATURES = [
    "brg_sin", "brg_cos", "alt_ft", "gs_kt", "vrate_fpm", "trk_off_sin", "trk_off_cos",
    "hour_sin", "hour_cos", "dow", "inbound", "landed_15", "landed_60", "holding",
    "arr_rwys", "south", "wind_u", "wind_v",
]


def matrix(df: pl.DataFrame) -> np.ndarray:
    return df.select(ETA_FEATURES).to_numpy().astype(np.float32)


def train_final(train: pl.DataFrame, val: pl.DataFrame):
    """Pick the tree count on `val`, then refit on train + val. Returns (model, n_trees)."""
    n = model.best_trees(matrix(train), train["asma_s"].to_numpy(), matrix(val), val["asma_s"].to_numpy())
    both = pl.concat([train, val])
    return model.refit(matrix(both), both["asma_s"].to_numpy(), n), n


def evaluate(m, base: EtaBaselines, df: pl.DataFrame) -> dict:
    y = df["asma_s"].to_numpy()
    return {
        "model": summary(y, m.predict(matrix(df))),
        "global_median": summary(y, base.global_median(df)),
        "sector_runway": summary(y, base.sector_runway(df)),
        "distance_speed": summary(y, base.distance_speed(df)),
    }


def band_edges(train: pl.DataFrame) -> tuple[float, float]:
    """Congestion bands: terciles of `inbound` (arrivals already inside the ring) on training days."""
    q = np.quantile(train["inbound"].to_numpy(), [1 / 3, 2 / 3])
    return float(q[0]), float(q[1])


def bands(df: pl.DataFrame, edges: tuple[float, float]) -> np.ndarray:
    v = df["inbound"].to_numpy()
    return np.where(v <= edges[0], "low", np.where(v <= edges[1], "mid", "high"))


def write_parity(onnx_path: Path, df: pl.DataFrame, out: Path, n: int = 64) -> None:
    """Inputs and ONNX Runtime outputs for `n` rows, for the Rust parity test."""
    sample = df.head(n)
    X = matrix(sample)
    expected = model.onnx_predict(onnx_path, X)
    rows = [[None if np.isnan(v) else float(v) for v in r] for r in X]
    Path(out).write_text(json.dumps({"features": ETA_FEATURES, "rows": rows, "expected": expected.tolist()}))
