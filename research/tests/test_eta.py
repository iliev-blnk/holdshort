import json

import numpy as np

from holdshort_research import eta, model
from holdshort_research.baselines import EtaBaselines
from tests.conftest import ETA_FEATURES, eta_frame


def test_feature_list_matches_rust():
    # The same list as ETA_FEATURES in crates/twin/src/features.rs.
    assert eta.ETA_FEATURES == ETA_FEATURES


def test_eta_train_eval_on_synthetic():
    train = eta_frame(1500, seed=1)
    val = eta_frame(400, seed=2)
    test = eta_frame(400, seed=3)
    m, n_trees = eta.train_final(train, val)
    assert n_trees >= 1
    res = eta.evaluate(m, EtaBaselines.fit(train), test)
    assert set(res) == {"model", "global_median", "sector_runway", "distance_speed"}
    assert res["model"]["mae"] < 0.5 * res["global_median"]["mae"]


def test_bands_split_by_training_terciles():
    train = eta_frame(300)
    lo, hi = eta.band_edges(train)
    assert lo <= hi
    labels = eta.bands(train, (lo, hi))
    assert set(labels) <= {"low", "mid", "high"}


def test_parity_file_round_trips(tmp_path):
    train = eta_frame(600, seed=4)
    m, _ = eta.train_final(train, eta_frame(200, seed=5))
    path = tmp_path / "eta.onnx"
    model.export_onnx(m, len(ETA_FEATURES), path)
    test = eta_frame(100, seed=6)
    out = tmp_path / "eta.json"
    eta.write_parity(path, test, out, n=64)
    p = json.loads(out.read_text())
    assert len(p["rows"]) == 64 and len(p["expected"]) == 64
    rows = np.array([[np.nan if v is None else v for v in r] for r in p["rows"]], dtype=np.float32)
    assert np.allclose(model.onnx_predict(path, rows), p["expected"], atol=1e-3)
    assert np.allclose(m.predict(rows), p["expected"], atol=1e-2)
