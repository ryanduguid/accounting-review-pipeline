from __future__ import annotations

import hashlib
from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.loader import SourceSnapshot


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"
TENANT = "Varrock Ventures Pty Ltd"


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
        "financial_year_reset",
        "period_variance",
        "subledger_reconciliation",
    }
    assert all(item.status == "REVIEW" for item in pack.exceptions)


def test_demo_pack_raises_exactly_the_expected_exceptions() -> None:
    # Exception count and membership are the product of a materiality engine, so
    # they are pinned account by account and in emitted order. Account 200 moves
    # -9,000.00 at 16.07%: it clears the percentage threshold but not the
    # $10,000 absolute one, and both must be met before an account is raised -
    # except against a nil prior balance, where there is no percentage to
    # compute and the absolute threshold decides alone (see
    # test_movement_from_a_nil_prior_balance_is_always_material_by_percentage).
    pack = _demo_pack()

    assert [(item.control, item.account_id) for item in pack.exceptions] == [
        ("account_mapping", "500"),
        # The demo fixtures compare 2026-06-30 against 2026-07-31, which
        # straddles the 30 June financial-year reset, so the pack carries the
        # crossing flag alongside the account-level exceptions.
        ("financial_year_reset", ""),
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
    # The exception must not claim a percentage threshold was cleared while its
    # own percentage_change column is blank: the reviewer would go looking for a
    # percentage the pack never computed.
    assert variances["777"].reason == (
        "YTD net balance moved beyond the configured absolute threshold; "
        "no percentage change could be computed from the prior YTD balance, "
        "so the percentage threshold was not tested."
    )
    # The mirror case, a balance falling to nil, has a defined percentage.
    assert variances["888"].percentage_change == Decimal("1")
    assert variances["888"].reason == "YTD net balance moved beyond both configured materiality thresholds."


def test_totals_that_balance_only_under_exact_decimal_arithmetic_pass(tmp_path: Path) -> None:
    # 0.10 + 0.20 == 0.30 is true in Decimal and false in binary floating point.
    # Parsing money through float would report this balanced trial balance as
    # BLOCKED, which is the failure the README's exact-Decimal claim rules out.
    rows = [
        "{date},{tenant},Assets,110,Trade Debtors,1100,0.00,0.00,0.10,0.00",
        "{date},{tenant},Assets,120,Other Debtors,1200,0.00,0.00,0.20,0.00",
        "{date},{tenant},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,0.30",
    ]
    prior = _write(tmp_path / "prior.csv", [row.format(date="2026-04-30", tenant=TENANT) for row in rows])
    current = _write(tmp_path / "current.csv", [row.format(date="2026-05-31", tenant=TENANT) for row in rows])

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
        "\n".join([header, *rows, "2026-07-31,Varrock Ventures Pty Ltd,Assets,777,Brand New Clearing,1770,0.00,0.00,0.00,0.00"]) + "\n",
        encoding="utf-8",
    )
    prior_header, *prior_rows = (EXAMPLES / "prior_trial_balance.csv").read_text(encoding="utf-8").splitlines()
    prior = tmp_path / "prior.csv"
    prior.write_text(
        "\n".join([prior_header, *prior_rows, "2026-06-30,Varrock Ventures Pty Ltd,Assets,888,Old Suspense,1880,0.00,0.00,0.00,0.00"]) + "\n",
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


def test_every_source_digest_is_bound_to_the_exact_bytes_parsed(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    source_paths = {
        "current_trial_balance": tmp_path / "current.csv",
        "prior_trial_balance": tmp_path / "prior.csv",
        "account_mapping": tmp_path / "mapping.csv",
        "subledger": tmp_path / "subledger.csv",
        "review_note": tmp_path / "review-note.json",
    }
    examples = {
        "current_trial_balance": EXAMPLES / "current_trial_balance.csv",
        "prior_trial_balance": EXAMPLES / "prior_trial_balance.csv",
        "account_mapping": EXAMPLES / "account_mapping.csv",
        "subledger": EXAMPLES / "subledger_balances.csv",
        "review_note": EXAMPLES / "review_note.json",
    }
    original_bytes = {}
    for role, destination in source_paths.items():
        content = examples[role].read_bytes()
        destination.write_bytes(content)
        original_bytes[role] = content

    replacement = b"replaced after the immutable read\n"
    real_text = SourceSnapshot.text

    def text_then_replace(
        snapshot: SourceSnapshot, *, label: str, encoding: str
    ) -> str:
        text = real_text(snapshot, label=label, encoding=encoding)
        snapshot.path.write_bytes(replacement)
        return text

    monkeypatch.setattr(SourceSnapshot, "text", text_then_replace)

    pack = review_close(
        current_path=source_paths["current_trial_balance"],
        prior_path=source_paths["prior_trial_balance"],
        mapping_path=source_paths["account_mapping"],
        subledger_path=source_paths["subledger"],
        acknowledgement_path=source_paths["review_note"],
        absolute_threshold=Decimal("10000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal("0.01"),
    )

    assert pack.source_hashes == {
        role: hashlib.sha256(content).hexdigest()
        for role, content in original_bytes.items()
    }
    assert all(path.read_bytes() == replacement for path in source_paths.values())
    # These values can only come from the original snapshots. The replacement
    # bytes are not valid inputs for any of the five loaders.
    assert pack.acknowledgement is not None
    assert pack.acknowledgement.reviewer_initials == "RD"
    assert [
        item.account_id for item in pack.exceptions if item.control == "account_mapping"
    ] == ["500"]
    assert [
        item.account_id
        for item in pack.exceptions
        if item.control == "subledger_reconciliation"
    ] == ["200"]


def test_period_comparison_rejects_different_tenants(tmp_path: Path) -> None:
    prior = tmp_path / "prior.csv"
    source = (EXAMPLES / "prior_trial_balance.csv").read_text(encoding="utf-8")
    prior.write_text(source.replace("Varrock Ventures Pty Ltd", "Falador Freight Pty Ltd"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="same tenant"):
        review_close(current_path=EXAMPLES / "current_trial_balance.csv", prior_path=prior)


def test_period_comparison_rejects_non_prior_date(tmp_path: Path) -> None:
    prior = tmp_path / "prior.csv"
    source = (EXAMPLES / "prior_trial_balance.csv").read_text(encoding="utf-8")
    prior.write_text(source.replace("2026-06-30", "2026-07-31"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="must be earlier"):
        review_close(current_path=EXAMPLES / "current_trial_balance.csv", prior_path=prior)


def _dated_pair(tmp_path: Path, prior_date: str, current_date: str) -> dict[str, Path]:
    """Two balanced one-account trial balances at caller-chosen report dates."""
    prior = _write(
        tmp_path / "prior.csv",
        [
            f"{prior_date},{TENANT},Assets,110,Trade Debtors,1100,0.00,0.00,100000.00,0.00",
            f"{prior_date},{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,100000.00",
        ],
    )
    current = _write(
        tmp_path / "current.csv",
        [
            f"{current_date},{TENANT},Assets,110,Trade Debtors,1100,0.00,0.00,100000.00,0.00",
            f"{current_date},{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,100000.00",
        ],
    )
    return {"current_path": current, "prior_path": prior}


@pytest.mark.parametrize(
    ("prior_date", "current_date"),
    [
        # Consecutive month ends inside FY2025 (1 July 2025 to 30 June 2026).
        ("2026-04-30", "2026-05-31"),
        # The last two month ends of the same financial year.
        ("2026-05-31", "2026-06-30"),
        # Both sides of a calendar-year end, which is mid financial year.
        ("2025-12-31", "2026-01-31"),
    ],
)
def test_report_dates_inside_one_financial_year_raise_no_reset_flag(
    tmp_path: Path, prior_date: str, current_date: str
) -> None:
    pack = review_close(**_dated_pair(tmp_path, prior_date, current_date))

    assert not [item for item in pack.exceptions if item.control == "financial_year_reset"]


@pytest.mark.parametrize(
    ("prior_date", "current_date"),
    [
        # 30 June against 31 July: the archetypal one-month straddle.
        ("2026-06-30", "2026-07-31"),
        # A whole financial year apart still crosses exactly one reset rule.
        ("2025-10-31", "2026-10-31"),
    ],
)
def test_report_dates_across_a_financial_year_reset_raise_a_review_flag(
    tmp_path: Path, prior_date: str, current_date: str
) -> None:
    # YTD figures for P&L accounts restart from nil on 1 July, so a YTD-vs-YTD
    # comparison across the reset weighs a full year against a month or two.
    # The engine has no section-aware rules to correct for that, so it must
    # flag the whole comparison rather than issue verdicts it cannot stand
    # behind.
    pack = review_close(**_dated_pair(tmp_path, prior_date, current_date))

    flags = [item for item in pack.exceptions if item.control == "financial_year_reset"]
    assert len(flags) == 1
    assert flags[0].status == "REVIEW"
    assert pack.status == "REVIEW"
    assert "different Australian financial years" in flags[0].reason
    assert "not meaningful" in flags[0].reason
    assert prior_date in flags[0].reason
    assert current_date in flags[0].reason


def test_exceptions_carry_the_mapped_review_group(tmp_path: Path) -> None:
    paths = _variance_pair(tmp_path, "150000.00")
    mapping = tmp_path / "mapping.csv"
    mapping.write_text(
        "AccountID,ReviewGroup\n110,Receivables\n900,Equity\n",
        encoding="utf-8",
    )

    pack = review_close(
        **paths,
        mapping_path=mapping,
        absolute_threshold=Decimal("10000"),
    )

    variances = {item.account_id: item for item in pack.exceptions if item.control == "period_variance"}
    assert variances["110"].review_group == "Receivables"
    assert variances["900"].review_group == "Equity"
    # An exception that names no account, such as the reset flag these
    # crossing dates raise, stays blank rather than borrowing a group.
    flags = [item for item in pack.exceptions if item.control == "financial_year_reset"]
    assert flags and flags[0].review_group == ""


def test_review_group_stays_blank_without_a_mapping(tmp_path: Path) -> None:
    pack = review_close(
        **_variance_pair(tmp_path, "150000.00"),
        absolute_threshold=Decimal("10000"),
    )

    assert pack.exceptions
    assert all(item.review_group == "" for item in pack.exceptions)


def test_an_unmapped_account_keeps_a_blank_review_group(tmp_path: Path) -> None:
    paths = _variance_pair(tmp_path, "150000.00")
    mapping = tmp_path / "mapping.csv"
    mapping.write_text("AccountID,ReviewGroup\n110,Receivables\n", encoding="utf-8")

    pack = review_close(
        **paths,
        mapping_path=mapping,
        absolute_threshold=Decimal("10000"),
    )

    unmapped = [item for item in pack.exceptions if item.control == "account_mapping"]
    assert [item.account_id for item in unmapped] == ["900"]
    assert unmapped[0].review_group == ""


def test_a_movement_from_a_nil_prior_ytd_balance_is_raised_by_the_absolute_gate_alone(tmp_path: Path) -> None:
    # Pin: 0 -> material. With a nil prior YTD balance no percentage exists,
    # and `percentage is None or percentage >= percentage_threshold` in the
    # engine deliberately treats the untestable gate as passed, so the
    # absolute gate decides alone. This is intended fail-closed behaviour,
    # not a bug: dropping the row because a percentage cannot be computed
    # would hide a first-funding of a clearing or suspense account.
    pack = review_close(
        **_variance_pair(tmp_path, "50000.00", prior_ytd="0.00"),
        absolute_threshold=Decimal("10000"),
        percentage_threshold=Decimal("0.10"),
    )

    raised = [item for item in pack.exceptions if item.control == "period_variance" and item.account_id == "110"]
    assert len(raised) == 1
    assert raised[0].percentage_change is None
    assert raised[0].difference == Decimal("50000.00")


def test_a_material_balance_falling_to_nil_is_still_raised(tmp_path: Path) -> None:
    # Pin: material -> 0. The prior balance is non-nil, so the percentage is
    # defined (exactly 1) and both gates fire in the ordinary way.
    pack = review_close(
        **_variance_pair(tmp_path, "0.00", prior_ytd="50000.00"),
        absolute_threshold=Decimal("10000"),
        percentage_threshold=Decimal("0.10"),
    )

    raised = [item for item in pack.exceptions if item.control == "period_variance" and item.account_id == "110"]
    assert len(raised) == 1
    assert raised[0].percentage_change == Decimal("1")
    assert raised[0].difference == Decimal("-50000.00")


def test_a_nil_to_nil_balance_raises_nothing(tmp_path: Path) -> None:
    # Pin: 0 -> 0. The difference is nil, so `difference != ZERO` in the
    # absolute gate fails first and the percentage-is-None arm never gets the
    # chance to raise a phantom exception for a dormant account.
    pack = review_close(
        **_variance_pair(tmp_path, "0.00", prior_ytd="0.00"),
        absolute_threshold=Decimal("10000"),
        percentage_threshold=Decimal("0.10"),
    )

    assert not [item for item in pack.exceptions if item.control == "period_variance"]


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
