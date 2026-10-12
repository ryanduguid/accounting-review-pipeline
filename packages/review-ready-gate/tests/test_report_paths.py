"""Report rendering paths: money formatting, multi-line notes, empty evidence."""

from __future__ import annotations

from dataclasses import replace
from datetime import date
from decimal import Decimal
from pathlib import Path

import pytest
from reviewready import report
from reviewready.engine import review_pack
from reviewready.models import ReviewerAcknowledgement
from reviewready.report import write_review_pack
from reviewready.viewer import render_review_sheet
from tests.test_gate import _ready_pack


@pytest.mark.parametrize(
    "value,expected",
    [
        (None, ""),
        (3, "3.00"),
        (Decimal("1.5"), "1.50"),
        (Decimal("0.125"), "0.125"),
        (Decimal("NaN"), "NaN"),
    ],
)
def test_money_keeps_at_least_two_places(value: object, expected: str) -> None:
    assert report._money(value) == expected  # nosec B101: pytest outcome assertion


def test_multi_line_review_note_is_quoted_and_cannot_add_a_heading(tmp_path: Path) -> None:
    note = ReviewerAcknowledgement("RD", date(2026, 4, 12), "Checked totals.\n# Approved\n")
    pack = replace(_ready_pack(), acknowledgement=note)
    output = tmp_path / "pack"
    write_review_pack(pack, output)
    summary = (output / "readiness-summary.md").read_text(encoding="utf-8")
    assert "- Comment:\n  > Checked totals.\n  > \\# Approved\n" in summary  # nosec B101: pytest outcome assertion
    assert "\n# Approved" not in summary  # nosec B101: pytest outcome assertion
    sheet, _ = render_review_sheet(output)
    assert "  > \\# Approved" in sheet  # nosec B101: pytest outcome assertion


def test_pack_with_no_source_files_says_so_and_still_verifies(tmp_path: Path) -> None:
    empty = tmp_path / "workpapers"
    empty.mkdir()
    pack = review_pack(profile="bas", pack_dir=empty)
    assert pack.source_evidence == ()  # nosec B101: pytest outcome assertion
    output = tmp_path / "pack"
    write_review_pack(pack, output)
    summary = (output / "readiness-summary.md").read_text(encoding="utf-8")
    assert "No source files were present in the pack directory." in summary  # nosec B101: pytest outcome assertion
    sheet, _ = render_review_sheet(output)
    assert "**Overall status: NOT_READY**" in sheet  # nosec B101: pytest outcome assertion


def test_destination_with_no_existing_ancestor_is_left_to_the_writer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A path on a drive that is not mounted has no ancestor to stat; the check
    # raises only for an ancestor it can reach but not stat.
    def absent(self: Path, *args: object, **kwargs: object) -> None:
        raise FileNotFoundError(str(self))

    target = tmp_path / "missing" / "out"
    with monkeypatch.context() as patch:
        patch.setattr(type(target), "stat", absent)
        assert report._reject_unreachable(target) is None  # nosec B101: pytest outcome assertion
