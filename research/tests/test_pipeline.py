import json
from datetime import date, timedelta

import numpy as np
import polars as pl

from holdshort_research.__main__ import run_all
from tests.conftest import eta_frame


def synthetic_data_dir(root):
    """Feature tables for every day 2026-09-01..10-08, shaped like `holdshort features` output."""
    d = date(2026, 9, 1)
    i = 0
    while d <= date(2026, 10, 8):
        day = d.isoformat()
        out = root / "LTFM" / "features" / f"date={day}"
        out.mkdir(parents=True)
        eta_frame(40, seed=i, dates=(day,)).write_parquet(out / "eta.parquet")
        rng = np.random.default_rng(i)
        base = np.array([5, 3, 2, 2, 4, 10, 20, 30, 35, 40, 38, 36, 34, 35, 37, 33, 30, 28, 30, 32, 25, 18, 12, 8])
        pl.DataFrame(
            {"date": [day] * 24, "hour": list(range(24)), "arrivals": (base + rng.integers(0, 4, 24)).tolist()},
            schema={"date": pl.String, "hour": pl.UInt8, "arrivals": pl.UInt32},
        ).write_parquet(out / "hourly.parquet")
        d += timedelta(days=1)
        i += 1
    return root


def test_run_all_on_synthetic_dir(tmp_path):
    data = synthetic_data_dir(tmp_path / "data")
    out = tmp_path / "models"
    rep = tmp_path / "report"
    res = run_all(data, out, rep)

    for f in ["eta.onnx", "hourly_h1.onnx", "hourly_h2.onnx", "hourly_h3.onnx", "meta.json", "parity/eta.json", "parity/hourly.json"]:
        assert (out / f).exists(), f
    meta = json.loads((out / "meta.json").read_text())
    assert len(meta["eta_features"]) == 18 and len(meta["hourly_features"]) == 9
    assert len(meta["profile"]) == 24
    assert meta["eta_global_median_s"] > 0
    hp = json.loads((out / "parity/hourly.json").read_text())
    assert len(hp["days"]) == 10 and all(len(c) == 24 for c in hp["counts"])
    assert set(hp["expected"]) == {"1", "2", "3"} and all(len(v) == 24 for v in hp["expected"].values())
    assert len(hp["rows"]["1"]) == 24 and len(hp["rows"]["1"][0]) == 9

    report = (rep / "REPORT.md").read_text()
    assert "Landing time at 40 nm" in report and "Arrivals per hour" in report
    assert (rep / "figures" / "eta_scatter.png").exists()
    assert res["eta"]["test"]["model"]["n"] == 7 * 40
