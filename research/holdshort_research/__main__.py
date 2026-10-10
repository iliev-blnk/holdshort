"""`python -m holdshort_research all --data ../data --out ../models --report .`

Trains both targets (tree count picked on the validation week, final model refit on
train + validation), scores them once on the test week, exports ONNX models plus
parity files for the Rust tests, and writes REPORT.md.
"""

import argparse
import json
from datetime import date, timedelta
from pathlib import Path

import numpy as np
import polars as pl

from . import eta, hourly, model, report
from .baselines import EtaBaselines
from .data import SPLIT, load_eta, load_hourly, split
from .metrics import summary


def _eta(df: pl.DataFrame, out: Path) -> dict:
    train, val, test = (split(df, p) for p in ("train", "val", "test"))
    m, n_trees = eta.train_final(train, val)
    fitted = pl.concat([train, val])
    base = EtaBaselines.fit(fitted)
    model.export_onnx(m, len(eta.ETA_FEATURES), out / "eta.onnx")
    eta.write_parity(out / "eta.onnx", test, out / "parity" / "eta.json")

    pred = m.predict(eta.matrix(test))
    y = test["asma_s"].to_numpy()
    edges = eta.band_edges(train)
    band = eta.bands(test, edges)
    per_band = {
        b: {k: summary(y[band == b], p[band == b]) for k, p in _eta_preds(m, base, test).items()}
        for b in ("low", "mid", "high")
        if (band == b).any()
    }
    days = test["date"].to_numpy()
    per_day = {
        d: {k: summary(y[days == d], p[days == d]) for k, p in _eta_preds(m, base, test).items()}
        for d in sorted(set(days))
    }
    importance = dict(zip(eta.ETA_FEATURES, m.booster_.feature_importance("gain").tolist()))
    return {
        "n_trees": n_trees,
        "rows": {p: len(split(df, p)) for p in SPLIT},
        "global_median_s": base.median,
        "band_edges": edges,
        "test": eta.evaluate(m, base, test),
        "per_band": per_band,
        "per_day": per_day,
        "importance": importance,
        "_scatter": (y, pred),
    }


def _eta_preds(m, base: EtaBaselines, df: pl.DataFrame) -> dict:
    return {
        "model": m.predict(eta.matrix(df)),
        "global_median": base.global_median(df),
        "sector_runway": base.sector_runway(df),
        "distance_speed": base.distance_speed(df),
    }


def _hourly(counts: pl.DataFrame, out: Path) -> dict:
    prof = hourly.profile(counts)
    res = {"profile": prof, "horizons": {}}
    for h in hourly.HORIZONS:
        f = hourly.features(counts, h, prof)
        train, val, test = (split(f, p) for p in ("train", "val", "test"))
        n = model.best_trees(hourly.matrix(train), train["y"].to_numpy(), hourly.matrix(val), val["y"].to_numpy())
        both = pl.concat([train, val])
        m = model.refit(hourly.matrix(both), both["y"].to_numpy(), n)
        model.export_onnx(m, len(hourly.HOURLY_FEATURES), out / f"hourly_h{h}.onnx")
        y = test["y"].to_numpy()
        pred = m.predict(hourly.matrix(test))
        scores = {"model": summary(y, pred)}
        scores.update({k: summary(y, v) for k, v in hourly.baselines(test).items()})
        res["horizons"][str(h)] = {"n_trees": n, "test": scores, "_series": (test["ts"].to_list(), y, pred, test["profile"].to_numpy())}
    _hourly_parity(counts, prof, out)
    return res


def _hourly_parity(counts: pl.DataFrame, prof: list[float], out: Path) -> None:
    """Raw counts for the last 10 days, plus Python's feature rows and ONNX outputs for the
    last day's 24 hours: Rust must rebuild the same rows from the counts."""
    last = max(counts["date"].to_list())
    end = date.fromisoformat(last)
    days = [(end - timedelta(days=9 - i)).isoformat() for i in range(10)]
    window = counts.filter(pl.col("date").is_in(days))
    by_day = {d: [0] * 24 for d in days}
    for d, hr, a in window.select("date", "hour", "arrivals").iter_rows():
        by_day[d][hr] = int(a)
    present = sorted(set(window["date"].to_list()))
    rows, expected = {}, {}
    for h in hourly.HORIZONS:
        f = hourly.features(window, h, prof).filter(pl.col("date") == last)
        X = hourly.matrix(f)
        rows[str(h)] = [[None if np.isnan(v) else float(v) for v in r] for r in X]
        expected[str(h)] = model.onnx_predict(out / f"hourly_h{h}.onnx", X).tolist()
    (out / "parity" / "hourly.json").write_text(
        json.dumps({"days": days, "present": present, "counts": [by_day[d] for d in days], "profile": prof, "rows": rows, "expected": expected})
    )


def run_all(data_dir: Path, out: Path, report_dir: Path) -> dict:
    out = Path(out)
    (out / "parity").mkdir(parents=True, exist_ok=True)
    eta_res = _eta(load_eta(Path(data_dir)), out)
    hourly_res = _hourly(load_hourly(Path(data_dir)), out)
    meta = {
        "eta_features": eta.ETA_FEATURES,
        "hourly_features": hourly.HOURLY_FEATURES,
        "profile": hourly_res["profile"],
        "split": SPLIT,
        "eta_global_median_s": eta_res["global_median_s"],
        "eta_n_trees": eta_res["n_trees"],
        "hourly_n_trees": {h: v["n_trees"] for h, v in hourly_res["horizons"].items()},
        "metrics": {
            "eta_test": eta_res["test"],
            "hourly_test": {h: v["test"] for h, v in hourly_res["horizons"].items()},
        },
        "trained": date.today().isoformat(),
    }
    (out / "meta.json").write_text(json.dumps(meta, indent=2))
    res = {"eta": eta_res, "hourly": hourly_res}
    report.write(Path(report_dir), res)
    return res


def main() -> None:
    p = argparse.ArgumentParser(prog="holdshort_research")
    p.add_argument("command", choices=["all"])
    p.add_argument("--data", type=Path, default=Path("../data"))
    p.add_argument("--out", type=Path, default=Path("../models"))
    p.add_argument("--report", type=Path, default=Path("."))
    a = p.parse_args()
    res = run_all(a.data, a.out, a.report)
    t = res["eta"]["test"]
    print(f"landing time: model MAE {t['model']['mae'] / 60:.2f} min vs best baseline "
          f"{min(v['mae'] for k, v in t.items() if k != 'model') / 60:.2f} min (n={t['model']['n']})")
    for h, v in res["hourly"]["horizons"].items():
        s = v["test"]
        print(f"arrivals/hour h={h}: model MAE {s['model']['mae']:.2f} vs best baseline "
              f"{min(x['mae'] for k, x in s.items() if k != 'model'):.2f}")


if __name__ == "__main__":
    main()
