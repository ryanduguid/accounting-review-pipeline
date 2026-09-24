"""The balance policy control: expected side, nil accounts and expected movement."""

from __future__ import annotations

from pathlib import Path

import pytest
from closecontrol.cli import main
from closecontrol.engine import review_close
from closecontrol.errors import DuplicateKeyError, SchemaError

HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n"
TENANT = "Lumbridge Fabricated Pty Ltd"


def _tb(path: Path, report_date: str, rows: list[str]) -> Path:
    path.write_text(HEADER + "".join(f"{report_date},{TENANT},{row}\n" for row in rows), encoding="utf-8")
    return path


def _inputs(tmp_path: Path, current_rows: list[str], policy: str) -> dict[str, Path]:
    prior = _tb(tmp_path / "prior.csv", "2026-07-31", [
        "Assets,BANK,Operating Bank,090,100.00,0.00,500.00,0.00",
        "Equity,CAP,Share Capital,970,0.00,100.00,0.00,500.00",
    ])
    current = _tb(tmp_path / "current.csv", "2026-08-31", current_rows)
    policy_path = tmp_path / "balance_policy.csv"
    policy_path.write_text("AccountID,ExpectedBalance,ExpectMovement\n" + policy, encoding="utf-8")
    return {"current_path": current, "prior_path": prior, "balance_policy_path": policy_path}


def _policy_items(pack):
    return [item for item in pack.exceptions if item.control == "balance_policy"]


def test_expected_side_nil_account_and_movement_each_raise_review(tmp_path: Path) -> None:
    pack = review_close(**_inputs(tmp_path, [
        # Overdrawn bank: the policy expects debit.
        "Assets,BANK,Operating Bank,090,0.00,700.00,0.00,200.00",
        # Suspense account the firm expects to clear to nil.
        "Liabilities,SUSP,Suspense,880,0.00,0.00,0.00,50.00",
        # Accrual with a balance but no movement this period.
        "Liabilities,ACCR,Accruals,820,0.00,0.00,0.00,300.00",
        "Equity,CAP,Share Capital,970,0.00,0.00,0.00,500.00",
        "Assets,DR,Debtors,110,700.00,0.00,1050.00,0.00",
    ], "BANK,debit,no\nSUSP,nil,no\nACCR,credit,yes\nCAP,credit,no\n"))

    reasons = {(item.account_id, item.reason) for item in _policy_items(pack)}
    assert reasons == {
        ("BANK", "The policy expects a debit balance; the account carries a credit balance."),
        ("SUSP", "The policy expects a nil balance; the account carries a credit balance."),
        ("ACCR", "The policy expects movement every period; the account has none in the current period."),
    }
    assert all(item.status == "REVIEW" for item in _policy_items(pack))
    assert "balance_policy" in pack.source_hashes
    assert "balance_policy" not in pack.controls_not_run
    # Firm-resolved: the policy raises no client question.
    assert not [query for query in pack.client_queries if query.control == "balance_policy"]


def test_a_declared_contra_account_and_a_nil_balance_pass(tmp_path: Path) -> None:
    pack = review_close(**_inputs(tmp_path, [
        "Assets,BANK,Operating Bank,090,100.00,0.00,600.00,0.00",
        # Accumulated depreciation is an asset with a credit balance, declared as such.
        "Assets,ACCDEP,Accumulated Depreciation,151,0.00,100.00,0.00,100.00",
        "Equity,CAP,Share Capital,970,0.00,0.00,0.00,500.00",
    ], "BANK,debit,yes\nACCDEP,credit,no\nCAP,any,no\nSUSP,nil,no\n"))

    assert _policy_items(pack) == []


def test_an_account_absent_from_the_trial_balance_has_no_movement(tmp_path: Path) -> None:
    pack = review_close(**_inputs(tmp_path, [
        "Assets,BANK,Operating Bank,090,0.00,0.00,500.00,0.00",
        "Equity,CAP,Share Capital,970,0.00,0.00,0.00,500.00",
    ], "WAGES,any,yes\n"))

    (item,) = _policy_items(pack)
    assert (item.tenant, item.account_id) == (TENANT, "WAGES")
    assert item.reason.endswith("and is absent from the current trial balance.")


@pytest.mark.parametrize(
    ("policy", "error"),
    [
        ("BANK,debits,no\n", SchemaError),
        ("BANK,debit,sometimes\n", SchemaError),
        ("BANK,debit,no\nBANK,credit,no\n", DuplicateKeyError),
        ("", SchemaError),
        ('BANK,debit,"no\n', SchemaError),
    ],
)
def test_a_malformed_policy_is_refused(tmp_path: Path, policy: str, error: type[Exception]) -> None:
    with pytest.raises(error):
        review_close(**_inputs(tmp_path, [
            "Assets,BANK,Operating Bank,090,0.00,0.00,500.00,0.00",
            "Equity,CAP,Share Capital,970,0.00,0.00,0.00,500.00",
        ], policy))


def test_the_cli_reads_the_policy_and_the_pack_still_verifies(tmp_path: Path) -> None:
    inputs = _inputs(tmp_path, [
        "Assets,BANK,Operating Bank,090,0.00,700.00,0.00,200.00",
        "Assets,DR,Debtors,110,700.00,0.00,700.00,0.00",
        "Equity,CAP,Share Capital,970,0.00,0.00,0.00,500.00",
    ], "BANK,debit,no\n")
    output = tmp_path / "pack"
    assert main([
        "review",
        "--current", str(inputs["current_path"]),
        "--prior", str(inputs["prior_path"]),
        "--balance-policy", str(inputs["balance_policy_path"]),
        "--output", str(output),
    ]) == 2
    assert "balance_policy" in (output / "exceptions.csv").read_text(encoding="utf-8")
    assert main(["view", "--pack-dir", str(output)]) == 0
