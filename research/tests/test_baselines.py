import numpy as np
import polars as pl

from holdshort_research.baselines import EtaBaselines
from tests.conftest import eta_frame


def test_global_median(eta_df):
    b = EtaBaselines.fit(eta_df)
    assert np.allclose(b.global_median(eta_df[:5]), np.median(eta_df["asma_s"]))


def test_sector_runway_falls_back_to_global():
    train = eta_frame(200).with_columns(sector=pl.lit(1, pl.UInt8), arr_rwys=pl.lit(2.0, pl.Float32))
    b = EtaBaselines.fit(train, min_group=20)
    seen = train[:1]
    unseen = train[:1].with_columns(sector=pl.lit(5, pl.UInt8))
    assert np.isclose(b.sector_runway(seen)[0], np.median(train["asma_s"]))
    assert np.isclose(b.sector_runway(unseen)[0], b.median)


def test_distance_speed_scales_by_train_ratio():
    # 40 nm at 300 kt = 480 s straight-line; actual is always 2x → ratio 2.
    train = eta_frame(100).with_columns(gs_kt=pl.lit(300.0, pl.Float32), asma_s=pl.lit(960.0))
    b = EtaBaselines.fit(train)
    test = train[:1].with_columns(gs_kt=pl.lit(240.0, pl.Float32))
    assert np.isclose(b.distance_speed(test)[0], 2 * 40 / 240 * 3600)


def test_distance_speed_without_gs_uses_sector():
    train = eta_frame(100)
    b = EtaBaselines.fit(train)
    test = train[:3].with_columns(gs_kt=pl.lit(float("nan"), pl.Float32))
    assert np.allclose(b.distance_speed(test), b.sector_runway(test))
