import numpy as np
import polars as pl

from holdshort_research import writeup_figures as W

T0 = 1_791_000_000_000
MIN = 60_000
BG = np.array([0x1B, 0x1F, 0x24]) / 255


def obs(rows):
    """rows: (ts_ms, hex, lat, lon)."""
    return pl.DataFrame(rows, schema=["ts_ms", "hex", "lat", "lon"], orient="row")


def test_flight_kinds_by_window():
    o = obs([(T0 + 10 * MIN, "a", 41.0, 28.7), (T0 + 190 * MIN, "a", 41.0, 28.7), (T0 + 360 * MIN, "a", 41.0, 28.7)])
    fl = pl.DataFrame(
        {
            "hex": ["a", "a"],
            "kind": ["arrival", "departure"],
            "first_ms": [T0, T0 + 180 * MIN],
            "last_ms": [T0 + 30 * MIN, T0 + 210 * MIN],
        }
    )
    assert W.flight_kinds(o, fl)["kind"].to_list() == ["arrival", "departure", "other"]


def test_flight_kinds_both_counts_as_arrival():
    o = obs([(T0 + 5 * MIN, "b", 41.0, 28.7)])
    fl = pl.DataFrame({"hex": ["b"], "kind": ["both"], "first_ms": [T0], "last_ms": [T0 + 10 * MIN]})
    assert W.flight_kinds(o, fl)["kind"].to_list() == ["arrival"]


def track(hex_, kind, pts):
    return pl.DataFrame(
        {
            "ts_ms": [T0 + i * 10_000 for i in range(len(pts))],
            "hex": [hex_] * len(pts),
            "lat": [p[0] for p in pts],
            "lon": [p[1] for p in pts],
            "kind": [kind] * len(pts),
        }
    )


def test_render_day_draws_tracks_in_their_colour():
    c = (41.0, 29.0)
    arr = track("a", "arrival", [(41.0, 28.5 + i * 0.05) for i in range(21)])  # west → east through the centre
    dep = track("d", "departure", [(41.4 - i * 0.04, 29.3) for i in range(21)])  # north → south, east of centre
    img = W.render_day(pl.concat([arr, dep]), c, 30.0, (200, 100))
    assert img.shape == (100, 200, 3)
    assert img.min() >= 0 and img.max() <= 1
    np.testing.assert_allclose(img[5, 5], BG, atol=1e-6)
    a = img[50, 100]  # on the arrival line
    assert a[2] > a[0]
    x = W.to_px(np.array([41.0]), np.array([29.3]), c, 30.0, (200, 100))[0][0]
    d = img[70, int(np.floor(x))]  # on the departure line, south of the arrival line
    assert d[0] > d[2]


def test_long_gaps_are_not_bridged():
    c = (41.0, 29.0)
    t = pl.DataFrame(
        {"ts_ms": [T0, T0 + 700_000], "hex": ["g", "g"], "lat": [41.0, 41.0], "lon": [28.6, 29.4], "kind": ["arrival", "arrival"]}
    )
    img = W.render_day(t, c, 30.0, (200, 100))
    np.testing.assert_allclose(img[50, 100], BG, atol=1e-6)
