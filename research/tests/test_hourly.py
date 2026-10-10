from datetime import datetime, timedelta

import numpy as np
import polars as pl

from holdshort_research import model
from holdshort_research.hourly import HOURLY_FEATURES, features, profile


def counts(days: list[str], value=lambda d, h: 10 * d + h) -> pl.DataFrame:
    rows = []
    for di, d in enumerate(days):
        for h in range(24):
            rows.append({"date": d, "hour": h, "arrivals": value(di, h)})
    df = pl.DataFrame(rows, schema={"date": pl.String, "hour": pl.UInt8, "arrivals": pl.UInt32})
    return df.with_columns(
        ts=pl.col("date").str.to_datetime("%Y-%m-%d") + pl.duration(hours=pl.col("hour").cast(pl.Int64))
    )


def row(df: pl.DataFrame, ts: datetime) -> dict:
    return df.filter(pl.col("ts") == ts).to_dicts()[0]


def test_lags_do_not_look_ahead():
    c = counts(["2026-09-01", "2026-09-02"], lambda d, h: 100 * d + h)
    f = features(c, horizon=2, profile=[0.0] * 24)
    r = row(f, datetime(2026, 9, 2, 10))
    # Issued at 09:00 for 10:00: the last completed hours are 08, 07, 06.
    assert (r["lag1"], r["lag2"], r["lag3"]) == (108, 107, 106)
    assert r["same_yday"] == 10
    assert r["y"] == 110
    assert list(f.select(HOURLY_FEATURES).columns) == HOURLY_FEATURES


def test_lags_skip_missing_days():
    c = counts(["2026-09-01", "2026-09-03"])
    f = features(c, horizon=1, profile=[0.0] * 24)
    r = row(f, datetime(2026, 9, 3, 0))
    assert np.isnan(r["lag1"]) and np.isnan(r["lag2"]) and np.isnan(r["lag3"])
    assert np.isnan(r["same_yday"])
    assert np.isnan(r["same_week"])


def test_profile_uses_train_only():
    c = counts(["2026-09-01", "2026-09-02", "2026-10-05"], lambda d, h: 1000 if d == 2 else 10 + d)
    p = profile(c)
    assert len(p) == 24
    assert np.allclose(p, 10.5)


def test_calendar_features():
    c = counts(["2026-09-01"])
    r = row(features(c, horizon=1, profile=list(range(24))), datetime(2026, 9, 1, 6))
    assert np.isclose(r["hour_sin"], 1.0) and abs(r["hour_cos"]) < 1e-9
    assert r["dow"] == 1  # Tuesday
    assert r["profile"] == 6


def test_onnx_matches_lightgbm(tmp_path):
    rng = np.random.default_rng(0)
    X = rng.normal(size=(500, 9)).astype(np.float32)
    X[::5, 3] = np.nan
    y = 3 * X[:, 0] + np.nan_to_num(X[:, 3])
    m = model.fit(X[:400], y[:400], X[400:], y[400:])
    path = tmp_path / "m.onnx"
    model.export_onnx(m, 9, path)
    assert np.allclose(model.onnx_predict(path, X[:20]), m.predict(X[:20]), atol=1e-4)
