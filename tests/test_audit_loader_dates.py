"""Both independently released loaders enforce the same calendar-date contract."""
import json
from pathlib import Path

import pytest
from tests.native_monthly_close import probe as close_probe
from tests.native_review_ready import probe as ready_probe


@pytest.mark.parametrize("probe", [close_probe, ready_probe])
@pytest.mark.parametrize("value", ["20260923", "2026-W39-3"])
def test_non_calendar_trial_balance_date_is_rejected(tmp_path: Path, probe, value: str) -> None:
    source = tmp_path / "tb.csv"
    source.write_text(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n"
        f"{value},Sample entity,Assets,one,Cash,100,0,0,0,0\n", encoding="utf-8"
    )
    assert "error" in probe({"op": "csv", "kind": "trial_balance", "path": str(source),
                             "hex": source.read_bytes().hex()})


@pytest.mark.parametrize("probe", [close_probe, ready_probe])
@pytest.mark.parametrize("value", ["20260923", "2026-W39-3"])
def test_non_calendar_review_date_is_rejected(tmp_path: Path, probe, value: str) -> None:
    source = tmp_path / "review.json"
    source.write_text(json.dumps({
        "reviewer_initials": "AB", "reviewed_on": value, "comment": "Fabricated review."
    }), encoding="utf-8")
    assert "error" in probe({"op": "review_note", "path": str(source), "hex": source.read_bytes().hex()})
