"""Approach anomaly detection: how far is each approach from the normal approaches to its runway?

The served detector is kNN on resampled paths (rows from crates/twin/src/approach.rs). Three
comparisons: the landing-time residual of sub-project 4, robust z on the metrics, and an
isolation forest. Everything is fitted on the training days, without labelled flights; the
percentile table comes from the validation week; the test week is scored once.
"""

import json
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import polars as pl
from sklearn.ensemble import IsolationForest

from . import eta, model
from .data import load_approach, load_eta, split

M = 32
METRICS = ["extra_nm", "time_min", "turn_deg", "level_min", "high_10nm_ft", "gs_4nm_kt", "xtk_final_m"]
RUNWAYS = ["34L", "35R"]
KS = [1, 5, 10, 20]
PATH_COLS = [f"p{i:02d}{c}" for i in range(M) for c in "xyh"]
DETECTORS = ["knn", "zmax", "iforest", "eta_resid"]
# |z| at or above this makes a metric a reason; at most this many reasons.
REASON_Z = 3.0
MAX_REASONS = 2
FLAG_PCT = 99


def scored(df: pl.DataFrame) -> pl.DataFrame:
    return df.filter(pl.col("unscored").is_null())


def labels(df: pl.DataFrame) -> np.ndarray:
    return (df["label_ga"] | df["label_hold"]).to_numpy()


def reference(df: pl.DataFrame) -> pl.DataFrame:
    """Scored rows without a label: the normal approaches."""
    s = scored(df)
    return s.filter(~(pl.col("label_ga") | pl.col("label_hold")))


def paths(df: pl.DataFrame) -> np.ndarray:
    return df.select(PATH_COLS).to_numpy().astype(np.float64).reshape(-1, M, 3)


def metrics(df: pl.DataFrame) -> np.ndarray:
    return df.select(METRICS).to_numpy().astype(np.float64)


def knn(ref: np.ndarray, q: np.ndarray, k: int, chunk: int = 32) -> tuple[np.ndarray, np.ndarray]:
    """Mean distance to the k nearest reference paths, and the index of the nearest.
    The distance between two paths is the mean over points of the Euclidean distance."""
    k = min(k, len(ref))
    scores = np.empty(len(q))
    nearest = np.empty(len(q), dtype=np.int64)
    for a in range(0, len(q), chunk):
        d = np.sqrt(((q[a : a + chunk, None] - ref[None]) ** 2).sum(-1)).mean(-1)
        nearest[a : a + chunk] = d.argmin(1)
        scores[a : a + chunk] = np.sort(d, axis=1)[:, :k].mean(1)
    return scores, nearest


@dataclass
class Robust:
    median: np.ndarray
    mad: np.ndarray  # scaled by 1.4826

    @classmethod
    def fit(cls, X: np.ndarray) -> "Robust":
        X = np.asarray(X, dtype=np.float64)
        med = np.nanmedian(X, axis=0)
        mad = 1.4826 * np.nanmedian(np.abs(X - med), axis=0)
        return cls(med, mad)

    def z(self, X: np.ndarray) -> np.ndarray:
        X = np.asarray(X, dtype=np.float64)
        ok = np.isfinite(self.mad) & (self.mad > 0)
        z = np.full(X.shape, np.nan)
        z[:, ok] = (X[:, ok] - self.median[ok]) / self.mad[ok]
        return z


def zmax(z: np.ndarray) -> np.ndarray:
    a = np.abs(z)
    return np.where(np.isnan(a).all(1), 0.0, np.nanmax(np.where(np.isnan(a), -np.inf, a), axis=1))


def reasons(z_row: np.ndarray) -> list[str]:
    a = np.abs(z_row)
    idx = [i for i in np.argsort(-np.where(np.isnan(a), -np.inf, a), kind="stable") if a[i] >= REASON_Z]
    return [METRICS[i] for i in idx[:MAX_REASONS]]


def quantiles(scores: np.ndarray) -> np.ndarray:
    return np.quantile(scores, np.linspace(0, 1, 101))


def percentile(s: float, q: np.ndarray) -> float:
    if s <= q[0]:
        return 0.0
    if s >= q[100]:
        return 100.0
    j = int(np.searchsorted(q, s, side="right")) - 1
    gap = q[j + 1] - q[j]
    return float(j + ((s - q[j]) / gap if gap > 0 else 0.0))


def average_precision(labels: np.ndarray, scores: np.ndarray) -> float:
    labels = np.asarray(labels, dtype=bool)
    if not labels.any():
        return float("nan")
    hit = labels[np.argsort(-np.asarray(scores), kind="stable")]
    prec = np.cumsum(hit) / np.arange(1, len(hit) + 1)
    return float(prec[hit].mean())


def recall_at(labels: np.ndarray, scores: np.ndarray, rate: float = 0.01) -> float:
    labels = np.asarray(labels, dtype=bool)
    if not labels.any():
        return float("nan")
    n = int(np.ceil(rate * len(labels)))
    top = np.argsort(-np.asarray(scores), kind="stable")[:n]
    return float(labels[top].sum() / labels.sum())


@dataclass
class RunwayModel:
    name: str
    ids: list[str]
    ref_f32: np.ndarray  # [N, 96] as read from parquet
    robust: Robust
    forest: IsolationForest
    quantiles: np.ndarray | None = None

    @property
    def ref(self) -> np.ndarray:
        return self.ref_f32.astype(np.float64).reshape(-1, M, 3)

    @property
    def threshold(self) -> float:
        return float(self.quantiles[FLAG_PCT])


@dataclass
class Model:
    k: int
    runways: dict[str, RunwayModel] = field(default_factory=dict)


def _filled(X: np.ndarray, med: np.ndarray) -> np.ndarray:
    return np.where(np.isnan(X), np.nan_to_num(med), X)


def fit(train: pl.DataFrame, k: int) -> Model:
    ref = reference(train)
    m = Model(k)
    for rw in RUNWAYS:
        r = ref.filter(pl.col("runway") == rw)
        if r.height == 0:
            continue
        X = metrics(r)
        rob = Robust.fit(X)
        forest = IsolationForest(n_estimators=300, random_state=0).fit(_filled(X, rob.median))
        m.runways[rw] = RunwayModel(
            name=rw,
            ids=[f"{d}/{f}" for d, f in zip(r["date"], r["flight_id"])],
            ref_f32=r.select(PATH_COLS).to_numpy().astype(np.float32),
            robust=rob,
            forest=forest,
        )
    return m


def score(m: Model, df: pl.DataFrame) -> pl.DataFrame:
    """Scored rows of df on the model's runways, with every detector's score (except the
    landing-time residual), and the percentile, flag and reasons once calibrated."""
    s = scored(df).with_row_index("_i")
    parts = []
    for rw, r in m.runways.items():
        part = s.filter(pl.col("runway") == rw)
        if part.height == 0:
            continue
        knn_s, nearest = knn(r.ref, paths(part), m.k)
        X = metrics(part)
        z = r.robust.z(X)
        cols = {
            "knn": knn_s,
            "nearest": nearest,
            "zmax": zmax(z),
            "iforest": -r.forest.score_samples(_filled(X, r.robust.median)),
            "reasons": [reasons(row) for row in z],
        }
        if r.quantiles is not None:
            cols["pct"] = np.array([percentile(v, r.quantiles) for v in knn_s])
            cols["flag"] = knn_s >= r.threshold
        else:
            cols["pct"] = np.full(part.height, np.nan)
            cols["flag"] = np.zeros(part.height, dtype=bool)
        parts.append(part.with_columns(**{c: pl.Series(c, v) for c, v in cols.items()}))
    return pl.concat(parts).sort("_i").drop("_i")


def calibrate(m: Model, val: pl.DataFrame) -> None:
    res = score(m, val)
    for rw, r in m.runways.items():
        r.quantiles = quantiles(res.filter(pl.col("runway") == rw)["knn"].to_numpy())


def _num(v: float):
    return None if not np.isfinite(v) else float(v)


def export(m: Model, out: Path) -> None:
    out = Path(out)
    meta = {
        "m": M,
        "k": m.k,
        "h_scale": 1000,
        "metrics": METRICS,
        "runways": [
            {
                "name": r.name,
                "n": len(r.ids),
                "ids": r.ids,
                "median": [_num(v) for v in r.robust.median],
                "mad": [_num(v) for v in r.robust.mad],
                "quantiles": [float(v) for v in r.quantiles],
                "threshold": r.threshold,
            }
            for r in m.runways.values()
        ],
    }
    (out / "approach.json").write_text(json.dumps(meta))
    np.concatenate([r.ref_f32 for r in m.runways.values()]).astype("<f4").tofile(out / "approach_ref.f32")


def eta_residual(df: pl.DataFrame, eta_df: pl.DataFrame, onnx_path: Path) -> np.ndarray:
    """|actual − predicted at 40 nm| in seconds; 0 for rows without a landing-time row."""
    pred = model.onnx_predict(onnx_path, eta.matrix(eta_df))
    e = eta_df.select("date", "flight_id", "asma_s").with_columns(pred=pl.Series(pred.astype(np.float64)))
    j = df.select("date", "flight_id").join(e, on=["date", "flight_id"], how="left", maintain_order="left")
    return np.nan_to_num(np.abs(j["asma_s"].to_numpy() - j["pred"].to_numpy()), nan=0.0)


def evaluate(res: pl.DataFrame) -> dict:
    y = labels(res)
    out = {
        d: {
            "ap": average_precision(y, res[d].to_numpy()),
            "recall_1pct": recall_at(y, res[d].to_numpy(), 0.01),
        }
        for d in DETECTORS
    }
    out["n"] = res.height
    out["n_labels"] = int(y.sum())
    out["flagged"] = int(res["flag"].sum())
    out["labelled_median_pct"] = float(np.median(res.filter(pl.Series(y))["pct"].to_numpy())) if y.any() else None
    return out


def write_parity(res: pl.DataFrame, out: Path, n_other: int = 40) -> None:
    flagged = res.filter(pl.col("flag"))
    other = res.filter(~pl.col("flag")).sort("date", "flight_id").head(n_other)
    rows = []
    for r in pl.concat([flagged, other]).iter_rows(named=True):
        rows.append(
            {
                "runway": r["runway"],
                "metrics": [_num(r[c]) for c in METRICS],
                "path": [float(r[c]) for c in PATH_COLS],
                "score": r["knn"],
                "pct": r["pct"],
                "flag": r["flag"],
                "nearest": r["nearest"],
                "reasons": r["reasons"],
            }
        )
    (Path(out) / "parity" / "approach.json").write_text(json.dumps({"rows": rows}))


def run(data_dir: Path, out: Path) -> dict:
    out = Path(out)
    (out / "parity").mkdir(parents=True, exist_ok=True)
    df = load_approach(Path(data_dir))
    eta_df = load_eta(Path(data_dir))
    train, val, test = (split(df, p) for p in ("train", "val", "test"))

    # k by validation AP (ties → smaller k), detectors pooled over both runways.
    by_k = {}
    for k in KS:
        res = score(fit(train, k), val)
        by_k[k] = average_precision(labels(res), res["knn"].to_numpy())
    best_k = max(KS, key=lambda k: (by_k[k], -k))

    m = fit(train, best_k)
    calibrate(m, val)
    export(m, out)
    res = score(m, test)
    res = res.with_columns(eta_resid=pl.Series(eta_residual(res, eta_df, out / "eta.onnx")))
    write_parity(res, out)
    per_day = {d: evaluate(res.filter(pl.col("date") == d)) for d in sorted(set(res["date"]))}
    return {
        "k": best_k,
        "val_ap_by_k": by_k,
        "rows": {p: scored(split(df, p)).height for p in ("train", "val", "test")},
        "reference": {rw: len(r.ids) for rw, r in m.runways.items()},
        "unscored": dict(df.filter(pl.col("unscored").is_not_null()).group_by("unscored").len().iter_rows()),
        "test": evaluate(res),
        "per_day": per_day,
        "_model": m,
        "_test": res,
        "_train": train,
    }
