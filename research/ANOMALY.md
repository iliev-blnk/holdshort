# Approach anomaly detection

Which approaches into Istanbul were unusual? Every arrival is compared with the normal approaches flown to the same runway, from the 40 nm ring to touchdown. The top 1% are flagged. Nothing in the detector knows what a hold or a go-around is; those events, found by the twin's rules, are used only to check it.

## Data

Approach rows from `holdshort features`: 15,245 scored approaches to 34L and 35R (train 9,028, validation 3,167, test 3,050). An approach is scored when it has a 40 nm entry, a confident touchdown, at least 20 points and no gap over 10 minutes. Unscored over the 38 days: no entry 6,127, lost on final 5,348, coverage 131, runway 81.

The reference is the training days' approaches without a hold or go-around: 34L 4,731, 35R 4,215.

Each approach is resampled at 32 points equally spaced along the path, in its runway's frame (nm along and across the centreline, height in thousands of feet).

## Detectors

- **kNN path distance (served).** The mean distance to the k nearest reference approaches. The distance between two approaches is the mean over the 32 points of the distance in (x, y, h). k = 5 was chosen on the validation week (AP by k: 1: 0.126, 5: 0.137, 10: 0.108, 20: 0.067). It was fixed as the served detector before the test week was scored, because it explains each flag with a real approach.
- **Robust z.** The largest |z| over seven metrics (extra distance, time, turns, level time, height at 10 nm, speed at 4 nm, cross-track on final), against the reference median and MAD.
- **Isolation forest** on the same seven metrics.
- **Landing-time residual.** |actual − predicted| from the sub-project 4 model at 40 nm.

## Results on the test week (2–8 October 2026)

3,050 approaches, 28 of them with a hold or go-around. Ranking all approaches by each detector's score:

| Detector | Average precision | Recall in the top 1% |
| --- | --- | --- |
| kNN path distance (served) | 0.208 | 0.393 |
| Robust z (max over metrics) | 0.069 | 0.000 |
| Isolation forest on metrics | 0.422 | 0.643 |
| Landing-time residual (sub-project 4) | 0.496 | 0.643 |

A random ranking would have an average precision of about 0.009.

**The served kNN detector is not the best at finding holds and go-arounds.** Landing-time residual (sub-project 4) ranks them higher. Holds and go-arounds are mostly long approaches: extra distance and extra time, which the metric detectors and the landing-time residual measure directly. The kNN distance compares shapes along the path, and resampling by distance squeezes a long hold into a short stretch of the path. The detector was fixed before the test week, so it stays; the review below shows what it does flag.

With the threshold from the validation week, kNN flagged 22 test approaches (0.7%). Labelled approaches had a median kNN percentile of 98.0.

![kNN scores](figures/anomaly_scores.png)

Per test day (kNN):

| Day | Approaches | Labelled | Flagged | AP | Recall in top 1% |
| --- | --- | --- | --- | --- | --- |
| 2026-10-02 | 484 | 11 | 9 | 0.153 | 0.182 |
| 2026-10-03 | 476 | 6 | 2 | 0.420 | 0.333 |
| 2026-10-04 | 426 | 4 | 4 | 0.359 | 0.500 |
| 2026-10-05 | 468 | 2 | 2 | 0.530 | 0.500 |
| 2026-10-06 | 409 | 0 | 2 | – | – |
| 2026-10-07 | 364 | 0 | 1 | – | – |
| 2026-10-08 | 423 | 5 | 2 | 0.654 | 0.800 |

## What the detector flags

Every flagged test approach is drawn below (red) over a sample of the reference approaches (grey), with its nearest normal approach dashed, and its height against distance to go. Each was put in a category by looking at the drawing; the categories are the author's judgement, not ground truth.

| Category | Approaches |
| --- | --- |
| entry from the north or north-east | 14 |
| go-around (twin label) | 6 |
| go-around the twin missed | 1 |
| unusual routing | 1 |

Most of the unlabelled flags entered the ring from the north or north-east. Only 1.1% of the reference approaches did, so the detector finds them far from everything it knows. The traffic flow is real but rare: WZZ398 is flagged three times at the same time of day.

The review also explains why kNN ranks go-arounds lower than the other detectors. 22 of the 8,946 reference approaches have a go-around shape in their resampled height profile (below 1,500 ft, then above 3,000 ft) but no twin label, and one flagged test approach is the same (a go-around the twin missed). Go-arounds the twin's rule misses stay in the reference and give the labelled go-arounds close neighbours.

| Flight | Day | Category | Note |
| --- | --- | --- | --- |
| THY3AY | 2026-10-02 | entry from the north or north-east | entered at 13°, round the east side, joined final at 17 nm; +89 nm |
| THY4MS | 2026-10-02 | go-around the twin missed | descends to about 800 ft, climbs back to 4,600 ft and lands from a second approach; no twin label |
| THY6317 | 2026-10-02 | entry from the north or north-east | entered at 62°, crossed north of the airport and came down the west side; +100 nm |
| THY727 | 2026-10-02 | entry from the north or north-east | entered at 53°, east downwind out to 28 nm south, level at 9,700 ft for about 40 nm |
| THY8JE | 2026-10-02 | go-around (twin label) | go-around and hold |
| THY4HE | 2026-10-02 | entry from the north or north-east | entered at 62°, wide east downwind |
| THY3UQ | 2026-10-02 | unusual routing | entered from the south (165°), crossed under the final to the west and joined at 17 nm |
| THY7PY | 2026-10-02 | go-around (twin label) |  |
| THY6543 | 2026-10-02 | entry from the north or north-east | entered at 8°, short join to final at 8 nm, 1,000 ft below a 3° path at 10 nm |
| THY184 | 2026-10-03 | go-around (twin label) |  |
| THY1JU | 2026-10-03 | entry from the north or north-east | entered at 4°, wide east downwind |
| WZZ398 | 2026-10-04 | entry from the north or north-east | entered at 4°, east side, joined at 13 nm; same flight at the same time on 10-05 and 10-08 |
| FIA5325 | 2026-10-04 | entry from the north or north-east | entered at 4°, wide east downwind, level at 9,700 ft |
| ROT261N | 2026-10-04 | entry from the north or north-east | entered at 6°, east side, joined at 13 nm |
| THY1CB | 2026-10-04 | go-around (twin label) |  |
| WZZ398 | 2026-10-05 | entry from the north or north-east | entered at 10°, east side, joined at 19 nm |
| BTI711 | 2026-10-05 | go-around (twin label) | also entered from the north |
| THY7AZ | 2026-10-06 | entry from the north or north-east | entered at 4°, cut straight to an 11 nm final, higher than normal on the way |
| THY5AW | 2026-10-06 | entry from the north or north-east | entered at 3°, joined at 11 nm |
| LOT135 | 2026-10-07 | entry from the north or north-east | entered at 4°, wide east downwind, joined at 21 nm |
| WZZ398 | 2026-10-08 | entry from the north or north-east | entered at 4°, east side, joined at 13 nm |
| THY4YH | 2026-10-08 | go-around (twin label) |  |

![Flagged approaches, page 1](figures/anomaly_gallery_1.png)
![Flagged approaches, page 2](figures/anomaly_gallery_2.png)

## Emergency squawk

IHAEM381 (4b8061) squawked 7700 on 2026-10-05. The twin classed it as overflight, so it has no approach score.

## Limitations

- The labels only cover large deviations (holds and go-arounds) found by the twin's own rules, so the numbers above say how well each detector finds those. Anything else the detector finds rests on the review.
- The labels are also used to keep holds and go-arounds out of the reference.
- North flow only: every day in the data landed on 34L/35R. South-flow approaches and the other runways are not scored.
- Coverage holes can look like unusual approaches. Lost-on-final arrivals (20% of arrivals) are not scored at all.
- 34L approaches sit about 500 ft below a 3° path at 10 nm with almost no spread, against about +40 ft on 35R. That is most likely an offset in the 34L threshold position from OurAirports. Per-runway statistics absorb it.

To reproduce: `holdshort features 2026-09-01..2026-10-08`, then `python -m holdshort_research approach --data ../data --out ../models --report .`, then `holdshort anomaly-check 2026-10-02..2026-10-08`.
