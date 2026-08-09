from __future__ import annotations

from pathlib import Path

import pytest

from closecontrol.errors import ControlInputError
from closecontrol.loader import load_canonical_tb, parse_money


ROOT = Path(__file__).resolve().parents[1]


def test_load_canonical_trial_balance_uses_stable_tenant_account_key() -> None:
    rows = load_canonical_tb(ROOT / "examples" / "current_trial_balance.csv")

    assert len(rows) == 7
    assert rows[0].key == ("Acme Demo Pty Ltd", "100")
    assert rows[0].ytd_net == parse_money("120000.00", field="test", row_number=1, path=Path("test"))


def test_loader_rejects_duplicate_control_key(tmp_path: Path) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    duplicate = tmp_path / "duplicate.csv"
    lines = source.splitlines()
    duplicate.write_text("\n".join(lines + [lines[1]]) + "\n", encoding="utf-8")

    with pytest.raises(ControlInputError, match="duplicate control key"):
        load_canonical_tb(duplicate)


@pytest.mark.parametrize("value", ["", "612,00", "1,2", "1 2", "1,234,56", "=1+1", "NaN"])
def test_amount_parser_rejects_ambiguous_or_formula_values(value: str) -> None:
    with pytest.raises(ControlInputError):
        parse_money(value, field="Debit", row_number=2, path=Path("input.csv"))


def test_empty_report_date_is_reported_as_empty_not_invalid_iso(tmp_path: Path) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    blank_date = tmp_path / "blank_date.csv"
    blank_date.write_text(source.replace("2026-07-31,Acme Demo Pty Ltd,Assets,100", ",Acme Demo Pty Ltd,Assets,100"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="empty ReportDate"):
        load_canonical_tb(blank_date)
