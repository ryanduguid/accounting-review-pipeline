from __future__ import annotations

from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"
TENANT = "Acme Demo Pty Ltd"


def _write(path: Path, rows: list[str]) -> Path:
    path.write_text("\n".join([HEADER, *rows]) + "\n", encoding="utf-8")
    return path


def _variance_pair(tmp_path: Path, current_ytd: str, prior_ytd: str = "100000.00") -> dict[str, Path]:
    """Two balanced one-account trial balances whose only difference is account 110's YTD position."""
    prior = _write(
        tmp_path / "prior.csv",
        [
            f"2026-06-30,{TENANT},Assets,110,Trade Debtors,1100,0.00,0.00,{prior_ytd},0.00",
            f"2026-06-30,{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,{prior_ytd}",
        ],
    )
    current = _write(
        tmp_path / "current.csv",
        [
            f"2026-07-31,{TENANT},Assets,110,Trade Debtors,1100,0.00,0.00,{current_ytd},0.00",
            f"2026-07-31,{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,{current_ytd}",
        ],
    )
    return {"current_path": current, "prior_path": prior}


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


def test_demo_pack_raises_exactly_the_expected_exceptions() -> None:
    # Exception count and membership are the product of a materiality engine, so
    # they are pinned account by account and in emitted order. Account 200 moves
    # -9,000.00 at 16.07%: it clears the percentage threshold but not the
    # $10,000 absolute one, and both must be met before an account is raised.
    pack = _demo_pack()

    assert [(item.control, item.account_id) for item in pack.exceptions] == [
        ("account_mapping", "500"),
        ("period_variance", "100"),
        ("period_variance", "110"),
        ("period_variance", "300"),
        ("period_variance", "500"),
        ("period_variance", "900"),
        ("subledger_reconciliation", "200"),
    ]


@pytest.mark.parametrize(
    ("current_ytd", "absolute", "percentage", "expected"),
    [
        # The absolute gate includes a movement sitting exactly on the threshold.
        ("110000.00", "10000", "0", True),
        ("109999.99", "10000", "0", False),
        ("110000.01", "10000", "0", True),
        # So does the percentage gate: 10,000 on 100,000 is exactly 0.10.
        ("110000.00", "0", "0.10", True),
        ("109999.99", "0", "0.10", False),
        # Both gates must be met. One alone is not an exception.
        ("109000.00", "10000", "0.05", False),
        ("115000.00", "10000", "0.90", False),
    ],
)
def test_variance_thresholds_are_inclusive_and_must_both_be_met(
    tmp_path: Path, current_ytd: str, absolute: str, percentage: str, expected: bool
) -> None:
    pack = review_close(
        **_variance_pair(tmp_path, current_ytd),
        absolute_threshold=Decimal(absolute),
        percentage_threshold=Decimal(percentage),
    )

    raised = [item for item in pack.exceptions if item.control == "period_variance" and item.account_id == "110"]
    assert bool(raised) is expected


@pytest.mark.parametrize(
    ("subledger_balance", "expected"),
    [("99999.99", False), ("99999.98", True), ("100000.01", False), ("100000.02", True)],
)
def test_reconciliation_tolerance_excludes_a_difference_exactly_on_the_tolerance(
    tmp_path: Path, subledger_balance: str, expected: bool
) -> None:
    subledger = tmp_path / "subledger.csv"
    subledger.write_text(
        f"Tenant,AccountID,SubledgerBalance\n{TENANT},110,{subledger_balance}\n",
        encoding="utf-8",
    )

    pack = review_close(
        **_variance_pair(tmp_path, "100000.00"),
        subledger_path=subledger,
        reconciliation_tolerance=Decimal("0.01"),
    )

    raised = [item for item in pack.exceptions if item.control == "subledger_reconciliation"]
    assert bool(raised) is expected


def test_movement_from_a_nil_prior_balance_is_always_material_by_percentage(tmp_path: Path) -> None:
    # A percentage change is undefined against a nil prior balance, so the
    # percentage gate must let it through rather than drop it. A clearing or
    # suspense account funded for the first time in the period is exactly what a
    # close reviewer needs to see.
    prior = _write(
        tmp_path / "prior.csv",
        [
            f"2026-06-30,{TENANT},Assets,100,Operating Bank,1000,0.00,0.00,100000.00,0.00",
            f"2026-06-30,{TENANT},Assets,777,New Clearing,1770,0.00,0.00,0.00,0.00",
            f"2026-06-30,{TENANT},Assets,888,Old Clearing,1880,0.00,0.00,50000.00,0.00",
            f"2026-06-30,{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,150000.00",
        ],
    )
    current = _write(
        tmp_path / "current.csv",
        [
            f"2026-07-31,{TENANT},Assets,100,Operating Bank,1000,0.00,0.00,100000.00,0.00",
            f"2026-07-31,{TENANT},Assets,777,New Clearing,1770,900000.00,0.00,900000.00,0.00",
            f"2026-07-31,{TENANT},Assets,888,Old Clearing,1880,0.00,50000.00,0.00,0.00",
            f"2026-07-31,{TENANT},Equity,900,Retained Earnings,3000,0.00,850000.00,0.00,1000000.00",
        ],
    )

    pack = review_close(
        current_path=current, prior_path=prior, absolute_threshold=Decimal("10000")
    )

    variances = {item.account_id: item for item in pack.exceptions if item.control == "period_variance"}
    assert set(variances) == {"777", "888", "900"}
    assert variances["777"].percentage_change is None
    assert variances["777"].prior_value == Decimal("0.00")
    assert variances["777"].difference == Decimal("900000.00")
    # The mirror case, a balance falling to nil, has a defined percentage.
    assert variances["888"].percentage_change == Decimal("1")


def test_totals_that_balance_only_under_exact_decimal_arithmetic_pass(tmp_path: Path) -> None:
    # 0.10 + 0.20 == 0.30 is true in Decimal and false in binary floating point.
    # Parsing money through float would report this balanced trial balance as
    # BLOCKED, which is the failure the README's exact-Decimal claim rules out.
    rows = [
        "{date},{tenant},Assets,110,Trade Debtors,1100,0.00,0.00,0.10,0.00",
        "{date},{tenant},Assets,120,Other Debtors,1200,0.00,0.00,0.20,0.00",
        "{date},{tenant},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,0.30",
    ]
    prior = _write(tmp_path / "prior.csv", [row.format(date="2026-06-30", tenant=TENANT) for row in rows])
    current = _write(tmp_path / "current.csv", [row.format(date="2026-07-31", tenant=TENANT) for row in rows])

    pack = review_close(current_path=current, prior_path=prior)

    assert pack.status == "PASS"
    assert pack.exceptions == ()


def test_unbalanced_current_trial_balance_blocks_review(tmp_path: Path) -> None:
    destination = tmp_path / "current.csv"
    source = (EXAMPLES / "current_trial_balance.csv").read_text(encoding="utf-8")
    destination.write_text(source.replace(",15000.00,0.00,120000.00", ",15000.01,0.00,120000.00"), encoding="utf-8")

    pack = review_close(current_path=destination, prior_path=EXAMPLES / "prior_trial_balance.csv")

    assert pack.status == "BLOCKED"
    assert any(item.control == "trial_balance_integrity" and item.status == "BLOCKED" for item in pack.exceptions)


@pytest.mark.parametrize(
    ("file_name", "old", "new", "period"),
    [
        ("current_trial_balance.csv", "15000.00,0.00,120000.00,0.00", "15000.00,0.00,120000.01,0.00", "Current"),
        ("prior_trial_balance.csv", "10000.00,0.00,105000.00,0.00", "10000.00,0.00,105000.01,0.00", "Prior"),
    ],
)
def test_unbalanced_ytd_columns_block_review_while_movement_still_balances(
    tmp_path: Path, file_name: str, old: str, new: str, period: str
) -> None:
    # Only the YTD pair is perturbed, so the movement control stays satisfied and
    # the YTD control is the sole thing standing between the reviewer and a
    # year-to-date position that does not balance.
    paths = {
        "current_path": EXAMPLES / "current_trial_balance.csv",
        "prior_path": EXAMPLES / "prior_trial_balance.csv",
    }
    destination = tmp_path / file_name
    destination.write_text(
        (EXAMPLES / file_name).read_text(encoding="utf-8").replace(old, new), encoding="utf-8"
    )
    paths["current_path" if file_name.startswith("current") else "prior_path"] = destination

    pack = review_close(**paths)

    integrity = [item for item in pack.exceptions if item.control == "trial_balance_integrity"]
    assert pack.status == "BLOCKED"
    assert [item.reason for item in integrity] == [f"{period} YTD debit and credit totals do not balance."]
    assert integrity[0].status == "BLOCKED"
    assert integrity[0].difference == Decimal("0.01")


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
