"""Engine controls on inputs the fixture packs do not exercise."""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest
from reviewready import engine
from reviewready.engine import review_pack
from reviewready.errors import GateInputError
from reviewready.models import (
    FINDING_MISSING_ARTEFACT,
    FINDING_TB_UNBALANCED,
    FINDING_TIEOUT_BREAK,
    FINDING_TIEOUT_UNSUPPORTED,
    TieOutRow,
)
from reviewready.profiles import Slot
from tests.support import copy_example_pack


def test_missing_pack_directory_is_refused(tmp_path: Path) -> None:
    with pytest.raises(GateInputError, match="pack directory does not exist"):
        review_pack(profile="bas", pack_dir=tmp_path / "absent")


def test_an_unknown_slot_kind_is_refused(tmp_path: Path) -> None:
    slot = Slot("notes", "notes.csv", True, "notes")
    with pytest.raises(GateInputError, match="unknown slot kind 'notes'"):
        engine._load_slot(slot, None, "bas")  # type: ignore[arg-type]


def test_unbalanced_prior_trial_balance_blocks(tmp_path: Path) -> None:
    pack = copy_example_pack("month-end-ready", tmp_path / "pack")
    prior = pack / "prior_trial_balance.csv"
    prior.write_text(
        prior.read_text(encoding="utf-8").replace("10000.00,0.00,105000.00", "10001.00,0.00,105000.00"),
        encoding="utf-8",
    )
    result = review_pack(profile="month_end", pack_dir=pack)
    assert result.status == "BLOCKED"  # nosec B101: pytest outcome assertion
    unbalanced = [item for item in result.findings if item.code == FINDING_TB_UNBALANCED]
    assert [item.slot for item in unbalanced] == ["prior_trial_balance"]  # nosec B101: pytest outcome assertion
    assert "Movement net 1.00 and YTD net 0.00" in unbalanced[0].reason  # nosec B101: pytest outcome assertion


def test_prior_trial_balance_without_a_current_one_skips_the_period_comparison(
    tmp_path: Path,
) -> None:
    pack = copy_example_pack("month-end-ready", tmp_path / "pack")
    (pack / "trial_balance.csv").unlink()
    result = review_pack(profile="month_end", pack_dir=pack)
    assert [(item.code, item.slot) for item in result.findings] == [  # nosec B101: pytest outcome assertion
        (FINDING_MISSING_ARTEFACT, "trial_balance")
    ]


def test_missing_open_items_file_runs_no_open_item_control(tmp_path: Path) -> None:
    pack = copy_example_pack("bas-ready", tmp_path / "pack")
    (pack / "open_items.csv").unlink()
    result = review_pack(profile="bas", pack_dir=pack)
    assert [(item.code, item.slot) for item in result.findings] == [  # nosec B101: pytest outcome assertion
        (FINDING_MISSING_ARTEFACT, "open_items")
    ]


@pytest.mark.parametrize("label", ["1A", "1B"])
def test_activity_statement_without_1a_or_1b_cannot_tie(tmp_path: Path, label: str) -> None:
    pack = copy_example_pack("bas-ready", tmp_path / "pack")
    statement = pack / "activity_statement.csv"
    statement.write_text(
        statement.read_text(encoding="utf-8").replace(f"{label},", "G1,"), encoding="utf-8"
    )
    result = review_pack(profile="bas", pack_dir=pack)
    assert result.status == "NOT_READY"  # nosec B101: pytest outcome assertion
    breaks = [item for item in result.findings if item.code == FINDING_TIEOUT_BREAK]
    assert [item.slot for item in breaks] == ["activity_statement"]  # nosec B101: pytest outcome assertion
    assert "missing label 1A or 1B" in breaks[0].reason  # nosec B101: pytest outcome assertion


def test_controls_skip_rows_of_another_type() -> None:
    """Each control reads only its own row type from what the loader returned."""
    findings: list = []
    engine._apply_open_item_controls({"open_items": ["not an item"]}, findings)
    engine._apply_bank_rec({"bank_rec": ["not a row"]}, findings, Decimal("0.01"))
    engine._apply_year_end_tieout({"tie_out_matrix": ["not a row"]}, findings)
    assert findings == []  # nosec B101: pytest outcome assertion


@pytest.mark.parametrize(
    "workpaper_ref,source_file", [("", "trial_balance.csv"), ("WP-100", "")]
)
def test_tied_line_without_its_evidence_is_unsupported(workpaper_ref: str, source_file: str) -> None:
    # The loader refuses an empty reference on a TIED row, so the control's own
    # guard is reached only by a caller that builds rows directly.
    findings: list = []
    row = TieOutRow("Cash", workpaper_ref, source_file, "TIED")
    engine._apply_year_end_tieout({"tie_out_matrix": [row]}, findings)
    assert [(item.code, item.slot) for item in findings] == [  # nosec B101: pytest outcome assertion
        (FINDING_TIEOUT_UNSUPPORTED, "tie_out_matrix")
    ]
    assert "marked TIED without a workpaper reference" in findings[0].reason  # nosec B101: pytest outcome assertion

