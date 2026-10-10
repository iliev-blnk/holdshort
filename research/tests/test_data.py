import numpy as np
import polars as pl

from holdshort_research.data import SPLIT, split
from holdshort_research.metrics import summary


def test_split_by_day_bounds():
    df = pl.DataFrame({"date": ["2026-08-31", "2026-09-01", "2026-09-24", "2026-09-25", "2026-10-01", "2026-10-02", "2026-10-08", "2026-10-09"]})
    assert split(df, "train")["date"].to_list() == ["2026-09-01", "2026-09-24"]
    assert split(df, "val")["date"].to_list() == ["2026-09-25", "2026-10-01"]
    assert split(df, "test")["date"].to_list() == ["2026-10-02", "2026-10-08"]
    assert set(SPLIT) == {"train", "val", "test"}


def test_summary_values():
    s = summary(np.array([0.0, 10.0, 20.0, 30.0]), np.array([1.0, 8.0, 20.0, 40.0]))
    assert s["n"] == 4
    assert s["mae"] == (1 + 2 + 0 + 10) / 4
    assert np.isclose(s["rmse"], np.sqrt((1 + 4 + 0 + 100) / 4))
    assert np.isclose(s["p90"], np.quantile([1, 2, 0, 10], 0.9))
