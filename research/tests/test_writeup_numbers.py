"""The write-up's headline numbers must appear in the reports they come from."""

from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
PAIRS = [
    ("1.39 min", "research/REPORT.md", "1.39"),
    ("1.90 min", "research/REPORT.md", "1.90"),
    ("3.82", "research/REPORT.md", "3.82"),
    ("0.105", "research/ANOMALY.md", "0.105"),
    ("0.496", "research/ANOMALY.md", "0.496"),
    ("0.423", "research/ANOMALY.md", "0.423"),
    ("−11.5%", "README.md", "−11.5%"),
    ("1,282", "README.md", "1,282"),
    ("0.208", "research/ANOMALY.md", "0.208"),
]


@pytest.mark.parametrize("in_writeup,source,in_source", PAIRS)
def test_number_is_in_writeup_and_source(in_writeup, source, in_source):
    assert in_writeup in (REPO / "docs/WRITEUP.md").read_text()
    assert in_source in (REPO / source).read_text()
