from __future__ import annotations

from decimal import Decimal
from pathlib import Path

from closecontrol.engine import review_close


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"


def _demo_pack():
    return review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        mapping_path=EXAMPLES / "account_mapping.csv",
        subledger_path=EXAMPLES / "subledger_balances.csv",
        acknowledgement_path=EXAMPLES / "review_note.json",
        absolute_threshold=Decimal("10000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal("0.01"),
    )


def test_demo_pack_is_review_not_approval() -> None:
    pack = _demo_pack()

    assert pack.status == "REVIEW"
    assert pack.acknowledgement is not None
    assert pack.acknowledgement.reviewer_initials == "RD"
    assert {item.control for item in pack.exceptions} == {
        "account_mapping",
        "period_variance",
        "subledger_reconciliation",
    }
    assert all(item.status == "REVIEW" for item in pack.exceptions)


def test_unbalanced_current_trial_balance_blocks_review(tmp_path: Path) -> None:
    destination = tmp_path / "current.csv"
    source = (EXAMPLES / "current_trial_balance.csv").read_text(encoding="utf-8")
    destination.write_text(source.replace(",15000.00,0.00,120000.00", ",15000.01,0.00,120000.00"), encoding="utf-8")

    pack = review_close(current_path=destination, prior_path=EXAMPLES / "prior_trial_balance.csv")

    assert pack.status == "BLOCKED"
    assert any(item.control == "trial_balance_integrity" and item.status == "BLOCKED" for item in pack.exceptions)


def test_source_hashes_change_when_source_changes(tmp_path: Path) -> None:
    copied = tmp_path / "current.csv"
    copied.write_text((EXAMPLES / "current_trial_balance.csv").read_text(encoding="utf-8"), encoding="utf-8")
    first = review_close(current_path=copied, prior_path=EXAMPLES / "prior_trial_balance.csv")
    copied.write_text(copied.read_text(encoding="utf-8").replace("Operating Bank", "Main Operating Bank"), encoding="utf-8")
    second = review_close(current_path=copied, prior_path=EXAMPLES / "prior_trial_balance.csv")

    assert first.source_hashes["current_trial_balance"] != second.source_hashes["current_trial_balance"]
