from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError


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


def test_account_metadata_change_raises_review_exception(tmp_path: Path) -> None:
    destination = tmp_path / "current.csv"
    source = (EXAMPLES / "current_trial_balance.csv").read_text(encoding="utf-8")
    destination.write_text(
        source.replace("100,Operating Bank,1000", "100,Renamed Bank,1900"),
        encoding="utf-8",
    )

    pack = review_close(current_path=destination, prior_path=EXAMPLES / "prior_trial_balance.csv")

    metadata = [item for item in pack.exceptions if item.control == "account_metadata"]
    assert len(metadata) == 1
    assert metadata[0].status == "REVIEW"
    assert metadata[0].account_id == "100"
    assert "account code and account name" in metadata[0].reason
    assert pack.status == "REVIEW"


def test_new_and_missing_accounts_raise_period_comparison_exceptions(tmp_path: Path) -> None:
    header, *rows = (EXAMPLES / "current_trial_balance.csv").read_text(encoding="utf-8").splitlines()
    # Zero-balance rows keep both trial balances exactly balanced, so the only
    # exceptions come from the account existing in one period but not the other.
    current = tmp_path / "current.csv"
    current.write_text(
        "\n".join([header, *rows, "2026-07-31,Acme Demo Pty Ltd,Assets,777,Brand New Clearing,1770,0.00,0.00,0.00,0.00"]) + "\n",
        encoding="utf-8",
    )
    prior_header, *prior_rows = (EXAMPLES / "prior_trial_balance.csv").read_text(encoding="utf-8").splitlines()
    prior = tmp_path / "prior.csv"
    prior.write_text(
        "\n".join([prior_header, *prior_rows, "2026-06-30,Acme Demo Pty Ltd,Assets,888,Old Suspense,1880,0.00,0.00,0.00,0.00"]) + "\n",
        encoding="utf-8",
    )

    pack = review_close(current_path=current, prior_path=prior)

    comparisons = {item.account_id: item for item in pack.exceptions if item.control == "period_comparison"}
    assert set(comparisons) == {"777", "888"}
    assert comparisons["777"].status == "REVIEW"
    assert comparisons["777"].reason == "New account is present in the current period only."
    assert comparisons["888"].status == "REVIEW"
    assert comparisons["888"].reason == "Account was present in the prior period but is absent from the current period."
    assert pack.status == "REVIEW"


def test_source_hashes_change_when_source_changes(tmp_path: Path) -> None:
    copied = tmp_path / "current.csv"
    copied.write_text((EXAMPLES / "current_trial_balance.csv").read_text(encoding="utf-8"), encoding="utf-8")
    first = review_close(current_path=copied, prior_path=EXAMPLES / "prior_trial_balance.csv")
    copied.write_text(copied.read_text(encoding="utf-8").replace("Operating Bank", "Main Operating Bank"), encoding="utf-8")
    second = review_close(current_path=copied, prior_path=EXAMPLES / "prior_trial_balance.csv")

    assert first.source_hashes["current_trial_balance"] != second.source_hashes["current_trial_balance"]


def test_period_comparison_rejects_different_tenants(tmp_path: Path) -> None:
    prior = tmp_path / "prior.csv"
    source = (EXAMPLES / "prior_trial_balance.csv").read_text(encoding="utf-8")
    prior.write_text(source.replace("Acme Demo Pty Ltd", "Other Tenant Pty Ltd"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="same tenant"):
        review_close(current_path=EXAMPLES / "current_trial_balance.csv", prior_path=prior)


def test_period_comparison_rejects_non_prior_date(tmp_path: Path) -> None:
    prior = tmp_path / "prior.csv"
    source = (EXAMPLES / "prior_trial_balance.csv").read_text(encoding="utf-8")
    prior.write_text(source.replace("2026-06-30", "2026-07-31"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="must be earlier"):
        review_close(current_path=EXAMPLES / "current_trial_balance.csv", prior_path=prior)


def test_review_note_cannot_predate_the_pack_it_claims_to_review(tmp_path: Path) -> None:
    note = tmp_path / "review.json"
    note.write_text(
        '{"reviewer_initials":"RD","reviewed_on":"2026-07-30","comment":"Reviewed."}',
        encoding="utf-8",
    )

    with pytest.raises(ControlInputError, match="cannot be earlier"):
        review_close(
            current_path=EXAMPLES / "current_trial_balance.csv",
            prior_path=EXAMPLES / "prior_trial_balance.csv",
            acknowledgement_path=note,
        )
