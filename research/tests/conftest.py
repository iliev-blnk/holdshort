import numpy as np
import polars as pl
import pytest

ETA_FEATURES = [
    "brg_sin", "brg_cos", "alt_ft", "gs_kt", "vrate_fpm", "trk_off_sin", "trk_off_cos",
    "hour_sin", "hour_cos", "dow", "inbound", "landed_15", "landed_60", "holding",
    "arr_rwys", "south", "wind_u", "wind_v",
]


def eta_frame(n: int, seed: int = 0, dates=("2026-09-01",)) -> pl.DataFrame:
    """A synthetic landing-time table shaped like eta.parquet, with a learnable signal:
    more inbound traffic means a longer approach."""
    rng = np.random.default_rng(seed)
    cols = {f: rng.normal(size=n).astype(np.float32) for f in ETA_FEATURES}
    cols["inbound"] = rng.integers(0, 20, n).astype(np.float32)
    cols["gs_kt"] = rng.uniform(250, 320, n).astype(np.float32)
    cols["arr_rwys"] = rng.integers(1, 3, n).astype(np.float32)
    asma = 840 + 30 * cols["inbound"] + rng.normal(0, 20, n)
    return pl.DataFrame(
        {
            "date": [dates[i % len(dates)] for i in range(n)],
            "flight_id": [f"f{i}" for i in range(n)],
            "t0_ms": np.arange(n, dtype=np.int64),
            "arr_rwy": ["35R"] * n,
            "sector": rng.integers(0, 8, n).astype(np.uint8),
            "dist_nm": np.full(n, 40.0),
            "asma_s": asma,
            **cols,
        }
    )


@pytest.fixture
def eta_df():
    return eta_frame(400)
