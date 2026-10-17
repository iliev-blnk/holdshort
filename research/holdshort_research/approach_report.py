"""Figures and research/ANOMALY.md for the approach anomaly detector."""

import csv
from collections import Counter
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
import polars as pl  # noqa: E402

from . import approach as A  # noqa: E402

PER_FIGURE = 12
REVIEW = "anomaly_review.csv"
# The first test-week run (2026-10-17), before the coverage fix below. Kept as history: the
# pipeline that produced it no longer exists, so these cannot be recomputed.
FIRST_RUN = {"k": 5, "n": 3050, "ap": 0.208, "recall_1pct": 0.393, "flagged": 22, "scored": 15245}
NAMES = {
    "knn": "kNN path distance (served)",
    "zmax": "Robust z (max over metrics)",
    "iforest": "Isolation forest on metrics",
    "eta_resid": "Landing-time residual (sub-project 4)",
}
REASON_TEXT = {
    "extra_nm": "nm flown beyond direct",
    "time_min": "min from 40 nm",
    "turn_deg": "° of turns",
    "level_min": "min level below 10,000 ft",
    "high_10nm_ft": "ft above 3° at 10 nm",
    "gs_4nm_kt": "kt at 4 nm",
    "xtk_final_m": "m off centreline on final",
}


def flights(data_dir: Path, dates: list[str], airport: str = "LTFM") -> pl.DataFrame:
    """Callsign, type and touchdown of the twin's flights on these days."""
    parts = []
    for d in dates:
        f = pl.read_parquet(Path(data_dir, airport, "twin", f"date={d}", "flights.parquet"))
        parts.append(f.select(pl.col("id").alias("flight_id"), "hex", "flight", "ac_type", "first_ms", "last_ms").with_columns(date=pl.lit(d)))
    return pl.concat(parts)


def squawk_7700(data_dir: Path, airport: str = "LTFM") -> pl.DataFrame:
    """Every (date, hex, first time) that squawked 7700."""
    files = sorted(Path(data_dir, airport, "parquet").glob("date=*/obs.parquet"))
    parts = [
        pl.read_parquet(f, columns=["ts_ms", "hex", "squawk"])
        .filter(pl.col("squawk") == "7700")
        .with_columns(date=pl.lit(f.parent.name[5:]))
        for f in files
    ]
    return pl.concat(parts).group_by("date", "hex").agg(pl.col("ts_ms").min()).sort("date")


def go_around_shape(p: np.ndarray) -> np.ndarray:
    """Resampled height profiles that go below 1,500 ft and later above 3,000 ft."""
    h = p[:, :, 2]
    return np.array([any((row[i + 1 :] > 3.0).any() for i in np.where(row < 1.5)[0]) for row in h])


def entry_bearing(p: np.ndarray, heading: float = 359.1) -> np.ndarray:
    """Bearing of the 40 nm entry from the threshold, degrees (34L/35R land on 359.1°)."""
    return (np.degrees(np.arctan2(p[:, 0, 1], p[:, 0, 0])) + heading) % 360


def _hhmm(ms: int) -> str:
    s = (ms // 1000) % 86400
    return f"{s // 3600:02d}:{s // 60 % 60:02d}"


def gallery(res: dict, data_dir: Path, fig_dir: Path) -> list[Path]:
    """One panel per test-week approach the served detector flags: plan view over the
    reference approaches, the nearest one dashed, and the height profile."""
    m: A.Model = res["_model"]
    t = res["_test"].filter(pl.col("flag")).sort("date", "touchdown_ms")
    info = flights(data_dir, sorted(set(t["date"])))
    t = t.join(info, on=["date", "flight_id"], how="left")
    rng = np.random.default_rng(0)
    out = []
    for start in range(0, t.height, PER_FIGURE):
        chunk = t.slice(start, PER_FIGURE)
        fig, axes = plt.subplots(len(chunk) // 4 + (len(chunk) % 4 > 0), 8, figsize=(24, 3.4 * (len(chunk) // 4 + 1)), squeeze=False)
        for ax in axes.flat:
            ax.set_visible(False)
        for i, r in enumerate(chunk.iter_rows(named=True)):
            rw = m.runways[r["runway"]]
            ref = rw.ref
            plan, prof = axes.flat[2 * i], axes.flat[2 * i + 1]
            plan.set_visible(True)
            prof.set_visible(True)
            for j in rng.choice(len(ref), min(400, len(ref)), replace=False):
                plan.plot(ref[j, :, 1], ref[j, :, 0], color="0.6", lw=0.4, alpha=0.15)
            p = np.array([[r[f"p{k:02d}{c}"] for c in "xyh"] for k in range(A.M)])
            nn = ref[r["nearest"]]
            plan.plot(nn[:, 1], nn[:, 0], color="k", lw=1.2, ls="--")
            plan.plot(p[:, 1], p[:, 0], color="tab:red", lw=1.6)
            plan.set_aspect("equal")
            plan.set_xlim(-45, 45)
            plan.set_ylim(-50, 25)
            plan.tick_params(labelsize=6)
            why = ", ".join(r["reasons"]) or "path shape"
            lab = "GA" if r["label_ga"] else ("HOLD" if r["label_hold"] else "")
            plan.set_title(f"{r['flight'] or r['flight_id']} {r['ac_type'] or ''} {r['date'][5:]} {_hhmm(r['touchdown_ms'])} {r['runway']}\n"
                           f"pct {r['pct']:.1f} {lab} · {why}", fontsize=7)
            length = r["extra_nm"] + float(np.hypot(p[0, 0], p[0, 1]))
            togo = length * (1 - np.arange(A.M) / (A.M - 1))
            ref_len = np.sqrt((np.diff(nn[:, :2], axis=0) ** 2).sum(1)).sum()
            prof.plot(ref_len * (1 - np.arange(A.M) / (A.M - 1)), nn[:, 2], color="k", lw=1, ls="--")
            prof.plot(togo, p[:, 2], color="tab:red", lw=1.4)
            prof.set_xlim(left=0)
            prof.invert_xaxis()
            prof.set_xlabel("nm to go", fontsize=6)
            prof.set_ylabel("kft", fontsize=6)
            prof.tick_params(labelsize=6)
        fig.tight_layout()
        path = fig_dir / f"anomaly_gallery_{start // PER_FIGURE + 1}.png"
        fig.savefig(path, dpi=110)
        plt.close(fig)
        out.append(path)
    return out


def score_figure(res: dict, fig_dir: Path) -> Path:
    t = res["_test"]
    fig, axes = plt.subplots(1, 2, figsize=(10, 3.4))
    for ax, rw in zip(axes, A.RUNWAYS):
        part = t.filter(pl.col("runway") == rw)
        y = A.labels(part)
        s = part["knn"].to_numpy()
        bins = np.linspace(0, np.quantile(s, 0.999), 60)
        ax.hist(s[~y], bins=bins, color="0.6", label="no label")
        ax.hist(s[y], bins=bins, color="tab:red", label="hold or go-around")
        ax.axvline(res["_model"].runways[rw].threshold, color="k", ls="--", lw=1, label="flag threshold")
        ax.set_yscale("log")
        ax.set_title(f"{rw}: kNN score, test week", fontsize=9)
        k = res["_model"].k
        ax.set_xlabel("distance to the nearest normal approach" if k == 1 else f"mean distance to the {k} nearest normal approaches")
        ax.legend(fontsize=7)
    fig.tight_layout()
    path = fig_dir / "anomaly_scores.png"
    fig.savefig(path, dpi=110)
    plt.close(fig)
    return path


def _review(report_dir: Path) -> list[dict]:
    p = report_dir / REVIEW
    if not p.exists():
        return []
    with p.open() as f:
        return list(csv.DictReader(f))


def _fmt(v) -> str:
    return "–" if v is None or (isinstance(v, float) and np.isnan(v)) else f"{v:.3f}"


def write(report_dir: Path, data_dir: Path, res: dict) -> None:
    report_dir = Path(report_dir)
    fig_dir = report_dir / "figures"
    fig_dir.mkdir(parents=True, exist_ok=True)
    pages = gallery(res, data_dir, fig_dir)
    score_figure(res, fig_dir)
    t = res["test"]
    flagged = set(res["_test"].filter(pl.col("flag"))["flight_id"])
    review = [r for r in _review(report_dir) if r["flight_id"] in flagged]
    missing = flagged - {r["flight_id"] for r in review}
    if missing:
        print(f"warning: {len(missing)} flagged approaches have no review row: {sorted(missing)}")
    cats = Counter(r["category"] for r in review)
    sq = squawk_7700(data_dir)
    test = res["_test"]

    L = ["# Approach anomaly detection", ""]
    L += [
        "Which approaches into Istanbul were unusual? Every arrival is compared with the normal approaches "
        "flown to the same runway, from the 40 nm ring to touchdown. The top 1% are flagged. Nothing in the "
        "detector knows what a hold or a go-around is; those events, found by the twin's rules, are used "
        "only to check it.",
        "",
        "## Data",
        "",
        f"Approach rows from `holdshort features`: {sum(res['rows'].values()):,} scored approaches to 34L and 35R "
        f"(train {res['rows']['train']:,}, validation {res['rows']['val']:,}, test {res['rows']['test']:,}). "
        "An approach is scored when it has a 40 nm entry, a confident touchdown, at least 20 points and no gap over 10 minutes. "
        "Unscored over the 38 days: "
        + ", ".join(f"{k.replace('_', ' ')} {v:,}" for k, v in sorted(res["unscored"].items(), key=lambda kv: -kv[1]))
        + ".",
        "",
        f"The reference is the training days' approaches without a hold or go-around: "
        + ", ".join(f"{rw} {n:,}" for rw, n in res["reference"].items())
        + ".",
        "",
        "Each approach is resampled at 32 points equally spaced along the path, in its runway's frame "
        "(nm along and across the centreline, height in thousands of feet).",
        "",
        "## Detectors",
        "",
        "- **kNN path distance (served).** The mean distance to the k nearest reference approaches. The distance between two "
        "approaches is the mean over the 32 points of the distance in (x, y, h). "
        f"k = {res['k']} was chosen on the validation week (AP by k: "
        + ", ".join(f"{k}: {v:.3f}" for k, v in res["val_ap_by_k"].items())
        + "). It was fixed as the served detector before the test week was scored, because it explains each flag with a real approach.",
        "- **Robust z.** The largest |z| over seven metrics (extra distance, time, turns, level time, height at 10 nm, speed at 4 nm, "
        "cross-track on final), against the reference median and MAD.",
        "- **Isolation forest** on the same seven metrics.",
        "- **Landing-time residual.** |actual − predicted| from the sub-project 4 model at 40 nm.",
        "",
        "## Results on the test week (2–8 October 2026)",
        "",
        f"{t['n']:,} approaches, {t['n_labels']} of them with a hold or go-around. Ranking all approaches by each detector's score:",
        "",
        "| Detector | Average precision | Recall in the top 1% |",
        "| --- | --- | --- |",
    ]
    for d in A.DETECTORS:
        L.append(f"| {NAMES[d]} | {_fmt(t[d]['ap'])} | {_fmt(t[d]['recall_1pct'])} |")
    top = test.sort("zmax", descending=True).head(int(np.ceil(0.01 * test.height)))
    near_const = int(top["zmax_metric"].is_in(["high_10nm_ft", "xtk_final_m"]).sum())
    L += [
        "",
        f"A random ranking would have an average precision of about {t['n_labels'] / t['n']:.3f}.",
        "",
        f"Robust z is held back by two nearly constant metrics. On 34L most aircraft are still level at 3,000 ft at 10 nm, "
        f"so the spread of the height at 10 nm is a few feet, and cross-track on final varies by a few metres on both runways. "
        f"Tiny differences there give huge z values: in {near_const} of the {top.height} top robust-z approaches the largest |z| "
        "is one of those two metrics. The reasons shown in the console therefore divide by at least a floor per metric "
        "(100 ft, 50 m, 5 kt, …); the robust-z detector above is scored as it was fitted.",
        "",
    ]
    best = max(A.DETECTORS, key=lambda d: t[d]["ap"])
    if best != "knn":
        L += [
            f"**The served kNN detector is not the best at finding holds and go-arounds.** {NAMES[best]} ranks them higher. "
            "Holds and go-arounds are mostly long approaches: extra distance and extra time, which the metric detectors and the "
            "landing-time residual measure directly. The kNN distance compares shapes along the path, and resampling by distance "
            "squeezes a long hold into a short stretch of the path. The detector was fixed before the test week, so it stays; the review "
            "below shows what it does flag.",
            "",
        ]
    f = FIRST_RUN
    L += [
        f"**A correction after the first test run.** The first run scored {f['n']:,} test approaches with k = {f['k']}: "
        f"kNN average precision {f['ap']:.3f}, recall {f['recall_1pct']:.3f}, {f['flagged']} flagged. The final code review then "
        "found a coverage bug: when the last point before the 40 nm entry was more than 10 minutes before the first point "
        "after it, the entry was interpolated across that gap (sometimes from an earlier flight of the same aircraft). "
        "Such approaches now count as coverage gaps, as the 10-minute rule intends. That removed "
        f"{f['scored'] - sum(res['rows'].values())} of the {f['scored']:,} scored approaches "
        f"({1 - sum(res['rows'].values()) / f['scored']:.1%}), the validation week then chose k = {res['k']} instead of {f['k']}, "
        "and the numbers above are from the corrected pipeline. "
        "Nothing else changed, and the fix was not chosen for its effect: it lowered the kNN numbers. That so small a change "
        f"moves them this much is itself a result: with {t['n_labels']} labelled test approaches, the kNN numbers are fragile.",
        "",
    ]
    L += [
        f"With the threshold from the validation week, kNN flagged {t['flagged']} test approaches "
        f"({t['flagged'] / t['n']:.1%}). Labelled approaches had a median kNN percentile of {t['labelled_median_pct']:.1f}.",
        "",
        "![kNN scores](figures/anomaly_scores.png)",
        "",
        "Per test day (kNN):",
        "",
        "| Day | Approaches | Labelled | Flagged | AP | Recall in top 1% |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for d, v in res["per_day"].items():
        L.append(f"| {d} | {v['n']} | {v['n_labels']} | {v['flagged']} | {_fmt(v['knn']['ap'])} | {_fmt(v['knn']['recall_1pct'])} |")
    ref = A.reference(res["_train"])
    ref = ref.filter(pl.col("runway").is_in(A.RUNWAYS))
    rp = A.paths(ref)
    shaped = int(go_around_shape(rp).sum())
    north = float(np.mean(entry_bearing(rp) < 90))
    L += ["", "## What the detector flags", ""]
    if review:
        L += [
            f"Every flagged test approach is drawn below (red) over a sample of the reference approaches (grey), with its "
            "nearest normal approach dashed, and its height against distance to go. Each was put in a category by looking at "
            "the drawing; the categories are the author's judgement, not ground truth.",
            "",
            "| Category | Approaches |",
            "| --- | --- |",
        ]
        L += [f"| {c} | {n} |" for c, n in cats.most_common()]
        L += [
            "",
            f"Most of the unlabelled flags entered the ring from the north or north-east. Only {north:.1%} of the reference "
            "approaches did, so the detector finds them far from everything it knows. The traffic flow is real but rare: "
            + ", ".join(f"{c} is flagged {'twice' if n == 2 else f'{n} times'}" for c, n in Counter(r["flight"] for r in review).items() if n > 1)
            + ", each time at the same time of day.",
            "",
            f"The review also explains why kNN ranks go-arounds lower than the other detectors. {shaped} of the {len(rp):,} reference "
            "approaches have a go-around shape in their resampled height profile (below 1,500 ft, then above 3,000 ft) but no "
            "twin label, and one flagged test approach is the same (a go-around the twin missed). Go-arounds the twin's rule misses "
            "stay in the reference and give the labelled go-arounds close neighbours.",
        ]
        L += ["", "| Flight | Day | Category | Note |", "| --- | --- | --- | --- |"]
        L += [f"| {r['flight']} | {r['date']} | {r['category']} | {r['note']} |" for r in review]
    else:
        L += ["Review pending."]
    L += [""] + [f"![Flagged approaches, page {i + 1}](figures/{p.name})" for i, p in enumerate(pages)]
    L += ["", "## Emergency squawk", ""]
    notes = []
    for d, hex_, ts in sq.iter_rows():
        twin = pl.read_parquet(Path(data_dir, "LTFM", "twin", f"date={d}", "flights.parquet")).filter(
            (pl.col("hex") == hex_) & (pl.col("first_ms") <= ts) & (pl.col("last_ms") >= ts)
        )
        if twin.height == 0:
            notes.append(f"{hex_} squawked 7700 on {d}; the twin has no flight for it at that time.")
            continue
        f = twin.row(0, named=True)
        hit = test.filter((pl.col("date") == d) & (pl.col("flight_id") == f["id"]))
        who = f"{f['flight'] or hex_} ({hex_})"
        if hit.height:
            notes.append(f"{who} squawked 7700 on {d}; its approach scored percentile {hit['pct'][0]:.1f}.")
        else:
            notes.append(f"{who} squawked 7700 on {d}. The twin classed it as {f['kind']}, so it has no approach score.")
    L += notes or ["No aircraft squawked 7700."]
    L += [
        "",
        "## Limitations",
        "",
        "- The labels only cover large deviations (holds and go-arounds) found by the twin's own rules, so the numbers above say how well "
        "each detector finds those. Anything else the detector finds rests on the review.",
        "- The labels are also used to keep holds and go-arounds out of the reference.",
        "- North flow only: every day in the data landed on 34L/35R. South-flow approaches and the other runways are not scored.",
        "- Coverage holes can look like unusual approaches. Lost-on-final arrivals (20% of arrivals) are not scored at all.",
        "- At 10 nm, most 34L traffic is still level at 3,000 ft (about 500 ft below a 3° path) and joins the glide path closer in, "
        "while 35R traffic is already on it. The height at 10 nm therefore means different things on the two runways; per-runway "
        "statistics keep them apart.",
        "",
        "To reproduce: `holdshort features 2026-09-01..2026-10-08`, then "
        "`python -m holdshort_research approach --data ../data --out ../models --report .`, then "
        "`holdshort anomaly-check 2026-10-02..2026-10-08`.",
        "",
    ]
    (report_dir / "ANOMALY.md").write_text("\n".join(L))
