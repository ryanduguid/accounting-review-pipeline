"""Both independently released loaders enforce the same calendar-date contract."""
import json
from pathlib import Path

import pytest
from closecontrol import loader as close
from reviewready import loader as ready


@pytest.mark.parametrize("loader", [close, ready])
@pytest.mark.parametrize("value", ["20260923", "2026-W39-3"])
def test_non_calendar_trial_balance_date_is_rejected(tmp_path: Path, loader, value: str) -> None:
    source = tmp_path / "tb.csv"
    source.write_text(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n"
        f"{value},Sample entity,Assets,one,Cash,100,0,0,0,0\n", encoding="utf-8"
    )
    with pytest.raises(loader.SchemaError):
        loader.load_canonical_tb(loader.SourceSnapshot.capture(source, label="Trial-balance file"))


@pytest.mark.parametrize("loader", [close, ready])
@pytest.mark.parametrize("value", ["20260923", "2026-W39-3"])
def test_non_calendar_review_date_is_rejected(tmp_path: Path, loader, value: str) -> None:
    source = tmp_path / "review.json"
    source.write_text(json.dumps({
        "reviewer_initials": "AB", "reviewed_on": value, "comment": "Fabricated review."
    }), encoding="utf-8")
    with pytest.raises(loader.SchemaError):
        loader.load_reviewer_acknowledgement(source)
