"""Claims in the write-up corrected in review: the wrong wording must not come back."""

from pathlib import Path

import pytest

TEXT = (Path(__file__).resolve().parents[2] / "docs/WRITEUP.md").read_text()
HTML = (Path(__file__).resolve().parents[2] / "console/writeup.html").read_text()

MUST_NOT = [
    "a simpler one",  # 0.496 is the landing-time model's error, not a simpler detector
    "Turkish airport operator",  # DHMI is the state airports authority; İGA runs the airport
    "airport operator's",
    "several rules further down",  # only the anomaly detector uses 10 minutes
    "Every number above",  # past runs and one-off measurements are not reproducible
    "GitHub's CDN resets",  # cause not established
    "Every free API I tried",  # only tested from Cloudflare's network
    "The top 1% are flagged",  # threshold is the validation week's 99th percentile
    "scikit-learn's gradient-boosting",  # it was skl2onnx's HistGradientBoosting converter
    "coverage, not logic",
    "─► weather",  # no such command
]
MUST = [
    "April 2026",  # the DHMI average is from another month
    "k = 5",
    "k = 1",
]


@pytest.mark.parametrize("phrase", MUST_NOT)
def test_corrected_wording_is_gone(phrase):
    assert phrase not in TEXT


@pytest.mark.parametrize("phrase", MUST)
def test_correction_is_stated(phrase):
    assert phrase in TEXT


def test_meta_description_has_no_self_praise():
    assert "honest" not in HTML.lower()
