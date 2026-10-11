"""REPORT.md and its figures."""

from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402

from .data import SPLIT  # noqa: E402

NAMES = {
    "model": "LightGBM",
    "global_median": "Global median",
    "sector_runway": "Median by sector × runways in use",
    "distance_speed": "Distance ÷ ground speed (scaled)",
    "persistence": "Last hour",
    "same_yday": "Same hour yesterday",
    "last7": "Same hour, last 7 days",
    "profile": "Hour-of-day average",
}
SLATE = {"arr": "#5aa9ff", "dep": "#ff9f43", "muted": "#8d97a1", "ink": "#262b31"}


def _min(s: float) -> str:
    return f"{s / 60:.2f}"


def _eta_table(scores: dict) -> str:
    lines = ["| Predictor | MAE (min) | RMSE (min) | P90 error (min) |", "| --- | --- | --- | --- |"]
    for k, s in scores.items():
        name = f"**{NAMES[k]}**" if k == "model" else NAMES[k]
        lines.append(f"| {name} | {_min(s['mae'])} | {_min(s['rmse'])} | {_min(s['p90'])} |")
    return "\n".join(lines)


def _best_baseline(scores: dict) -> tuple[str, dict]:
    return min(((k, v) for k, v in scores.items() if k != "model"), key=lambda kv: kv[1]["mae"])


def _figures(fig_dir: Path, res: dict) -> None:
    fig_dir.mkdir(parents=True, exist_ok=True)
    plt.rcParams.update({"font.size": 10, "axes.spines.top": False, "axes.spines.right": False})

    y, pred = res["eta"]["_scatter"]
    fig, ax = plt.subplots(figsize=(5, 5))
    ax.scatter(y / 60, pred / 60, s=4, alpha=0.35, color=SLATE["arr"])
    lim = [min(y.min(), pred.min()) / 60, max(y.max(), pred.max()) / 60]
    ax.plot(lim, lim, color=SLATE["muted"], lw=1)
    ax.set(xlabel="Actual time from 40 nm to touchdown (min)", ylabel="Predicted (min)", title="Landing time, test week")
    fig.tight_layout()
    fig.savefig(fig_dir / "eta_scatter.png", dpi=150)
    plt.close(fig)

    bands = res["eta"]["per_band"]
    keys = ["model", "sector_runway", "global_median"]
    fig, ax = plt.subplots(figsize=(6, 3.5))
    x = np.arange(len(bands))
    for i, k in enumerate(keys):
        ax.bar(x + (i - 1) * 0.27, [bands[b][k]["mae"] / 60 for b in bands], 0.27, label=NAMES[k],
               color=[SLATE["arr"], SLATE["muted"], SLATE["ink"]][i])
    ax.set_xticks(x, [f"{b} traffic" for b in bands])
    ax.set(ylabel="MAE (min)", title="Error by congestion (arrivals already inside 40 nm)")
    ax.legend(frameon=False, fontsize=8)
    fig.tight_layout()
    fig.savefig(fig_dir / "eta_by_band.png", dpi=150)
    plt.close(fig)

    imp = sorted(res["eta"]["importance"].items(), key=lambda kv: kv[1])
    fig, ax = plt.subplots(figsize=(5, 5))
    ax.barh([k for k, _ in imp], [v for _, v in imp], color=SLATE["arr"])
    ax.set(xlabel="Total gain", title="What the landing-time model uses")
    fig.tight_layout()
    fig.savefig(fig_dir / "eta_importance.png", dpi=150)
    plt.close(fig)

    ts, y, pred, prof = res["hourly"]["horizons"]["1"]["_series"]
    fig, ax = plt.subplots(figsize=(10, 3.5))
    ax.plot(ts, y, color=SLATE["ink"], lw=1.2, label="Actual")
    ax.plot(ts, pred, color=SLATE["arr"], lw=1.2, label="LightGBM, 1 h ahead")
    ax.plot(ts, prof, color=SLATE["muted"], lw=1, ls="--", label="Hour-of-day average")
    ax.set(ylabel="Arrivals per hour", title="Arrivals per hour, test week")
    ax.legend(frameon=False, fontsize=8, ncols=3)
    fig.autofmt_xdate()
    fig.tight_layout()
    fig.savefig(fig_dir / "hourly_test_week.png", dpi=150)
    plt.close(fig)


def write(report_dir: Path, res: dict) -> None:
    report_dir.mkdir(parents=True, exist_ok=True)
    _figures(report_dir / "figures", res)
    e = res["eta"]
    t = e["test"]
    bk, bv = _best_baseline(t)
    gain = 1 - t["model"]["mae"] / bv["mae"]
    day_rows = "\n".join(
        f"| {d} | {s['model']['n']} | {_min(s['model']['mae'])} | {_min(_best_baseline(s)[1]['mae'])} |"
        for d, s in e["per_day"].items()
    )
    band_rows = "\n".join(
        f"| {b} | {s['model']['n']} | {_min(s['model']['mae'])} | {_min(s['sector_runway']['mae'])} | {_min(s['global_median']['mae'])} |"
        for b, s in e["per_band"].items()
    )
    lo, hi = e["band_edges"]

    h_lines = ["| Horizon | LightGBM MAE | Last hour | Same hour yesterday | Same hour, last 7 days | Hour-of-day average |", "| --- | --- | --- | --- | --- | --- |"]
    for h, v in res["hourly"]["horizons"].items():
        s = v["test"]
        h_lines.append(
            f"| {h} h | **{s['model']['mae']:.2f}** | {s['persistence']['mae']:.2f} | {s['same_yday']['mae']:.2f} | {s['last7']['mae']:.2f} | {s['profile']['mae']:.2f} |"
        )
    h1 = res["hourly"]["horizons"]["1"]["test"]
    hk, hv = _best_baseline(h1)
    beats = [h for h, v in res["hourly"]["horizons"].items() if v["test"]["model"]["mae"] < _best_baseline(v["test"])[1]["mae"]]
    if len(beats) == len(res["hourly"]["horizons"]):
        hourly_verdict = (f"The model beats every baseline at every horizon; one hour ahead its error is "
                          f"{h1['model']['mae']:.2f} arrivals per hour against {hv['mae']:.2f} ({NAMES[hk]}).")
    else:
        hourly_verdict = (f"**The model does not reliably beat a simple baseline.** One hour ahead its error is "
                          f"{h1['model']['mae']:.2f} arrivals per hour against {hv['mae']:.2f} for {NAMES[hk].lower()}; "
                          f"it is ahead of the best baseline only at horizon(s) {', '.join(beats) or 'none'}, and by "
                          f"very little. Hub schedules repeat from day to day, so the recent same-hour average already "
                          f"carries most of what can be predicted from positions alone. Beating it would need schedule "
                          f"data, which this project does not use. Since the test week has now been seen, any change "
                          f"made to chase this baseline would need a new, unseen test week.")

    text = f"""# holdshort research: two short-term forecasts for Istanbul Airport

Generated by `python -m holdshort_research all` from the twin's feature tables
(`holdshort features`). Days are split in time order: train {SPLIT['train'][0]}..{SPLIT['train'][1]},
validation {SPLIT['val'][0]}..{SPLIT['val'][1]} (tree count), test {SPLIT['test'][0]}..{SPLIT['test'][1]}
(scored once). Final models are refit on train + validation. Rows: {e['rows']['train']} train,
{e['rows']['val']} validation, {e['rows']['test']} test arrivals.

## Landing time at 40 nm

When an arrival first comes within 40 nm of LTFM, how long until it lands? This is the ASMA time
(Eurocontrol's Arrival Sequencing and Metering Area) that an arrival manager has to plan.

{_eta_table(t)}

The model's mean error is **{_min(t['model']['mae'])} min** against **{_min(bv['mae'])} min** for the best
baseline ({NAMES[bk]}), {gain:.0%} lower.

![Predicted vs actual](figures/eta_scatter.png)

By congestion (arrivals already inside the ring when the flight enters: low ≤ {lo:.0f}, mid ≤ {hi:.0f},
high above, cut at the training terciles):

| Traffic | Flights | LightGBM MAE (min) | Sector × runways (min) | Global median (min) |
| --- | --- | --- | --- | --- |
{band_rows}

![Error by congestion](figures/eta_by_band.png)

By test day:

| Day | Flights | LightGBM MAE (min) | Best baseline MAE (min) |
| --- | --- | --- | --- |
{day_rows}

![Feature importance](figures/eta_importance.png)

## Arrivals per hour

At the top of each hour, how many arrivals will land in each of the next three hours? Mean
absolute error in arrivals per hour on the test week:

{chr(10).join(h_lines)}

{hourly_verdict}

![Arrivals per hour](figures/hourly_test_week.png)

## Limitations

- **One airport, {len(e['per_day'])} test days, about five weeks in all.** No seasonal or
  schedule changes are in the data, and there is one week of test. The numbers say how well this
  works on these weeks, not across a year.
- **Coverage.** The twin only knows what the open ADS-B feeders see. Arrivals whose touchdown
  time was estimated (lost on final) are left out of the landing-time rows. Coverage also got
  worse on 6–8 October, inside the test week, and was weak on 5 and 10–12 September (training
  days with 145–243 usable arrivals instead of the usual ~450). Those days are kept as they are;
  their hourly counts are below the real traffic.
- **`inbound` is slightly optimistic.** It counts aircraft inside 40 nm that later land; a live
  system sees the same aircraft but cannot be certain each one will land.
- **No schedules or flight plans.** Everything comes from positions, METAR and the twin's own
  events. An operational arrival manager also knows the planned sequence and the runway in use
  ahead of time.
- **Hourly counts are touchdowns the twin detected**, not official movements (about 10% below
  DHMI's published averages).
"""
    (report_dir / "REPORT.md").write_text(text)
