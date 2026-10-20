"""A balanced BAS export still has to belong to the declared period."""
import csv

import pytest
from reviewready.engine import review_pack
from reviewready.models import FINDING_PERIOD_ORDER
from tests.support import copy_example_pack


@pytest.mark.parametrize("posting_date,expected", [
    ("2026-04-01", "NOT_READY"),
    ("2026-03-31", "READY"),
    ("2026-01-01", "READY"),
])
def test_bas_refuses_later_gst_postings(tmp_path, posting_date, expected):
    pack_dir = copy_example_pack("bas-ready", tmp_path / "pack")
    source = pack_dir / "gst_control_gl.csv"
    with source.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        fields = reader.fieldnames
        rows = list(reader)
    for row in rows:
        row["Date"] = posting_date
    with source.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)
    result = review_pack(profile="bas", pack_dir=pack_dir)
    assert result.status == expected
    assert any(item.code == FINDING_PERIOD_ORDER and item.slot == "gst_control_gl"
               for item in result.findings) == (expected == "NOT_READY")
