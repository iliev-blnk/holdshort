import json
import math

import numpy as np
import polars as pl
import pytest

from holdshort_research import approach as A
from holdshort_research.data import split


def frame(offsets, runway="35R", dates=None, labels=None, metrics=None):
    """Approach rows whose paths are straight lines at a constant cross-track offset (nm)."""
    n = len(offsets)
    cols = {
        "date": dates or ["2026-09-01"] * n,
        "flight_id": [f"f{i}" for i in range(n)],
        "touchdown_ms": np.arange(n, dtype=np.int64),
        "runway": [runway] * n,
        "unscored": [None] * n,
        "label_ga": labels if labels is not None else [False] * n,
        "label_hold": [False] * n,
    }
    for j, m in enumerate(A.METRICS):
        cols[m] = np.asarray(metrics[:, j] if metrics is not None else np.zeros(n), dtype=np.float32)
    x = np.linspace(-40, 0, A.M)
    for i in range(A.M):
        cols[f"p{i:02d}x"] = np.full(n, x[i], dtype=np.float32)
        cols[f"p{i:02d}y"] = np.asarray(offsets, dtype=np.float32)
        cols[f"p{i:02d}h"] = np.full(n, -x[i] * 0.3184, dtype=np.float32)
    return pl.DataFrame(cols)


def test_path_columns_and_shape():
    df = frame([0.0, 1.0])
    assert A.PATH_COLS[:3] == ["p00x", "p00y", "p00h"] and A.PATH_COLS[-1] == "p31h"
    p = A.paths(df)
    assert p.shape == (2, 32, 3) and p.dtype == np.float64
    assert p[1, 5, 1] == 1.0


def test_knn_hand_computed():
    ref = A.paths(frame([0.0, 1.0, 3.0]))
    q = A.paths(frame([0.5]))
    s, nearest = A.knn(ref, q, 1)
    assert s[0] == pytest.approx(0.5) and nearest[0] == 0
    s, _ = A.knn(ref, q, 2)
    assert s[0] == pytest.approx(0.5)
    s, _ = A.knn(ref, q, 3)
    assert s[0] == pytest.approx((0.5 + 0.5 + 2.5) / 3)


def test_knn_chunks_agree():
    rng = np.random.default_rng(1)
    ref = rng.normal(size=(50, 32, 3))
    q = rng.normal(size=(70, 32, 3))
    s1, n1 = A.knn(ref, q, 5, chunk=7)
    s2, n2 = A.knn(ref, q, 5, chunk=1000)
    np.testing.assert_allclose(s1, s2)
    np.testing.assert_array_equal(n1, n2)


def test_percentile_edges():
    q = np.linspace(0, 100, 101)
    assert A.percentile(-1.0, q) == 0.0
    assert A.percentile(50.5, q) == pytest.approx(50.5)
    assert A.percentile(200.0, q) == 100.0
    flat = q.copy()
    flat[40:43] = 40.0
    assert A.percentile(40.0, flat) == pytest.approx(42.0)


def test_robust_and_reasons():
    rng = np.random.default_rng(0)
    X = rng.normal(size=(500, 7)).astype(np.float32)
    rob = A.Robust.fit(X)
    row = np.median(X, axis=0).copy()
    row[2] += 10 * rob.mad[2]
    assert A.reasons(rob.z(row[None])[0]) == ["turn_deg"]
    row[5] += 20 * rob.mad[5]
    assert A.reasons(rob.z(row[None])[0]) == ["gs_4nm_kt", "turn_deg"]
    row[0] += 30 * rob.mad[0]
    assert A.reasons(rob.z(row[None])[0]) == ["extra_nm", "gs_4nm_kt"]
    assert A.reasons(rob.z(np.median(X, axis=0)[None])[0]) == []


def test_zero_mad_is_ignored():
    X = np.zeros((10, 7), dtype=np.float32)
    X[:, 1] = np.arange(10)
    rob = A.Robust.fit(X)
    row = np.zeros((1, 7))
    row[0, 0] = 5.0
    z = rob.z(row)
    assert np.isnan(z[0, 0]) and not np.isnan(z[0, 1])
    assert A.reasons(z[0]) == []
    assert A.zmax(np.full((1, 7), np.nan))[0] == 0.0
    assert A.zmax_metric(np.array([[np.nan, 2.0, -5.0, 0, 0, 0, 0], [np.nan] * 7])) == ["turn_deg", None]


def test_reference_excludes_labels_and_unscored():
    df = frame([0.0, 1.0, 2.0], labels=[False, True, False]).with_columns(
        unscored=pl.Series([None, None, "coverage"], dtype=pl.String)
    )
    ref = A.reference(df)
    assert ref["flight_id"].to_list() == ["f0"]
    assert A.labels(df).tolist() == [False, True, False]


def test_average_precision_and_recall():
    labels = np.array([1, 0, 1, 0], dtype=bool)
    scores = np.array([0.9, 0.8, 0.7, 0.1])
    assert A.average_precision(labels, scores) == pytest.approx((1 + 2 / 3) / 2)
    assert A.recall_at(labels, scores, 0.25) == pytest.approx(0.5)
    assert A.recall_at(labels, scores, 0.75) == pytest.approx(1.0)
    assert math.isnan(A.average_precision(np.zeros(3, bool), np.ones(3)))
    assert math.isnan(A.recall_at(np.zeros(3, bool), np.ones(3)))


def test_split_by_day():
    df = frame([0.0, 0.0, 0.0], dates=["2026-09-24", "2026-09-25", "2026-10-02"])
    assert split(df, "train").height == 1 and split(df, "val").height == 1 and split(df, "test").height == 1


def test_fit_and_export_round_trip(tmp_path):
    rng = np.random.default_rng(0)
    off = rng.normal(0, 0.2, 60)
    train = frame(off, dates=["2026-09-01"] * 60, metrics=rng.normal(size=(60, 7)))
    model = A.fit(train, k=5)
    val = frame(rng.normal(0, 0.2, 40), dates=["2026-09-25"] * 40, metrics=rng.normal(size=(40, 7)))
    A.calibrate(model, val)
    A.export(model, tmp_path)
    meta = json.loads((tmp_path / "approach.json").read_text())
    assert meta["m"] == 32 and meta["k"] == 5 and meta["metrics"] == A.METRICS
    rw = meta["runways"][0]
    assert rw["name"] == "35R" and rw["n"] == 60 and len(rw["quantiles"]) == 101
    assert rw["threshold"] == rw["quantiles"][99]
    raw = np.fromfile(tmp_path / "approach_ref.f32", dtype="<f4")
    assert raw.size == 60 * 96
    np.testing.assert_array_equal(raw.reshape(60, 96), train.select(A.PATH_COLS).to_numpy())
    res = A.score(model, val)
    assert set(res.columns) >= {"knn", "pct", "flag", "nearest", "reasons"}
    assert res["flag"].sum() <= 2


def test_go_around_shape_and_entry_bearing():
    from holdshort_research.approach_report import entry_bearing, go_around_shape

    p = np.zeros((2, 32, 3))
    p[:, :, 2] = np.linspace(10, 0, 32)
    p[1, 10, 2] = 1.0  # dips below 1,500 ft, then is back above 3,000 ft
    assert go_around_shape(p).tolist() == [False, True]
    p[0, 0, :2] = [0.0, 40.0]  # 40 nm to the right of a 359.1° runway: east
    p[1, 0, :2] = [40.0, 0.0]  # straight ahead: north
    np.testing.assert_allclose(entry_bearing(p), [89.1, 359.1])


def test_reasons_use_a_mad_floor():
    # Near-constant metrics (34L level at 3,000 ft; tiny cross-track spread) must not turn
    # sub-resolution differences into reasons; zmax keeps the raw MAD.
    rng = np.random.default_rng(0)
    m = rng.normal(size=(60, 7))
    m[:, 0] *= 10  # extra_nm spread well above its 1 nm floor
    m[:, 4] = -509.0 + rng.normal(0, 5, 60)
    m[:, 6] = 15.0 + rng.normal(0, 1, 60)
    train = frame(rng.normal(0, 0.2, 60), metrics=m)
    model = A.fit(train, k=5)
    rw = model.runways["35R"]
    assert rw.reason_robust.mad[4] == 100.0 and rw.reason_robust.mad[6] == 50.0
    assert rw.reason_robust.mad[0] == rw.robust.mad[0]
    q = np.median(m, axis=0)
    q[4] += 40.0  # 40 ft: many raw MADs away, well under the floor
    q[6] += 13.0
    res = A.score(model, frame([0.0], metrics=q[None]))
    assert res["reasons"][0].to_list() == []
    assert res["zmax"][0] > 3
    A.calibrate(model, frame(rng.normal(0, 0.2, 40), dates=["2026-09-25"] * 40, metrics=rng.normal(size=(40, 7))))
    import tempfile, pathlib
    with tempfile.TemporaryDirectory() as d:
        A.export(model, pathlib.Path(d))
        meta = json.loads((pathlib.Path(d) / "approach.json").read_text())
    assert meta["runways"][0]["mad"][4] == 100.0
