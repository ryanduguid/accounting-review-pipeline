from __future__ import annotations

import csv
import json
from decimal import Decimal
from pathlib import Path

from closecontrol.cli import main
from closecontrol.engine import CloseReviewPack, review_close
from closecontrol.models import ExceptionItem
from closecontrol.report import write_review_pack


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"


def test_report_files_are_deterministic_and_csv_text_is_formula_safe(tmp_path: Path) -> None:
    pack = CloseReviewPack(
        status="REVIEW",
        current_report_dates=("2026-07-31",),
        prior_report_dates=("2026-06-30",),
        source_hashes={"current_trial_balance": "abc"},
        absolute_threshold=Decimal("1000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal("0.01"),
        exceptions=(
            ExceptionItem(
                control="period_variance",
                status="REVIEW",
                tenant="=untrusted",
                account_id="@123",
                account_code="-1000",
                account_name="+unsafe",
                current_value=Decimal("1000"),
                prior_value=Decimal("0"),
                difference=Decimal("1000"),
                threshold=Decimal("100"),
                percentage_change=None,
                reason="Demo only.",
                reviewer_action="Review.",
            ),
        ),
        acknowledgement=None,
    )

    first = write_review_pack(pack, tmp_path / "one")
    second = write_review_pack(pack, tmp_path / "two")

    assert first["json"].read_text(encoding="utf-8") == second["json"].read_text(encoding="utf-8")
    assert first["summary"].read_text(encoding="utf-8") == second["summary"].read_text(encoding="utf-8")
    with first["exceptions"].open(encoding="utf-8", newline="") as source:
        row = next(csv.DictReader(source))
    assert row["tenant"] == "'=untrusted"
    assert row["account_id"] == "'@123"
    assert row["account_code"] == "'-1000"
    assert row["account_name"] == "'+unsafe"


def test_cli_writes_review_pack_and_returns_attention_exit_code(tmp_path: Path) -> None:
    output = tmp_path / "pack"
    exit_code = main(
        [
            "review",
            "--current", str(EXAMPLES / "current_trial_balance.csv"),
            "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
            "--mapping", str(EXAMPLES / "account_mapping.csv"),
            "--subledger", str(EXAMPLES / "subledger_balances.csv"),
            "--review-note", str(EXAMPLES / "review_note.json"),
            "--absolute-threshold", "10000",
            "--percentage-threshold", "0.10",
            "--output", str(output),
        ]
    )

    assert exit_code == 2
    payload = json.loads((output / "close-review-pack.json").read_text(encoding="utf-8"))
    assert payload["overall_status"] == "REVIEW"
    assert payload["acknowledgement"]["effect"].startswith("Acknowledgement is evidence")


def test_cli_returns_one_for_malformed_input(tmp_path: Path) -> None:
    bad = tmp_path / "bad.csv"
    bad.write_text("wrong,header\n", encoding="utf-8")

    exit_code = main([
        "review", "--current", str(bad), "--prior", str(EXAMPLES / "prior_trial_balance.csv"), "--output", str(tmp_path / "out")
    ])

    assert exit_code == 1
