"""Variance drivers: the report-date gate and clean-up of a partly written output."""

from __future__ import annotations

import dataclasses
from pathlib import Path

import pytest
from closecontrol import drivers
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.report import write_review_pack

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"
TRANSACTIONS = EXAMPLES / "variance_transactions.csv"


def _pack(tmp_path: Path, **changes) -> Path:
    pack = review_close(current_path=EXAMPLES / "current_trial_balance.csv",
                        prior_path=EXAMPLES / "prior_trial_balance.csv")
    output = tmp_path / "pack"
    write_review_pack(dataclasses.replace(pack, **changes), output)
    return output


def test_a_pack_with_two_current_report_dates_is_refused(tmp_path: Path) -> None:
    pack = _pack(tmp_path, current_report_dates=("2026-07-30", "2026-07-31"))
    with pytest.raises(ControlInputError, match="one prior and one current report date"):
        drivers.variance_drivers(pack, TRANSACTIONS, currency="AUD")


def test_a_failed_write_removes_what_it_created(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    result = drivers.variance_drivers(_pack(tmp_path), TRANSACTIONS, currency="AUD")
    output = tmp_path / "drivers"
    real_open = Path.open

    def open_until_csv(self: Path, *args, **kwargs):  # noqa: ANN002, ANN003
        if self.name == "variance-drivers.csv":
            raise OSError("disk full")
        return real_open(self, *args, **kwargs)

    monkeypatch.setattr(Path, "open", open_until_csv)
    with pytest.raises(OSError, match="disk full"):
        drivers.write_drivers(result, output)
    assert not output.exists()  # nosec B101: pytest outcome assertion
