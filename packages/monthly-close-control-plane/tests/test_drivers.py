"""Variance drivers: ranked transactions behind each period_variance exception."""

from __future__ import annotations

import csv
import json
from pathlib import Path

import pytest
from closecontrol.cli import main
from closecontrol.drivers import variance_drivers
from closecontrol.errors import ControlInputError, DuplicateKeyError

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"
TRANSACTIONS = EXAMPLES / "variance_transactions.csv"
HEADER = "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\n"


@pytest.fixture
def pack(tmp_path: Path) -> Path:
    output = tmp_path / "pack"
    assert main([
        "review",
        "--current", str(EXAMPLES / "current_trial_balance.csv"),
        "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
        "--output", str(output),
    ]) == 2
    return output


def _account(result: dict, account_id: str) -> dict:
    (account,) = [item for item in result["accounts"] if item["account_id"] == account_id]
    return account


def test_drivers_rank_the_window_and_report_what_they_leave_unexplained(pack: Path) -> None:
    result = variance_drivers(pack, TRANSACTIONS, top=2)

    debtors = _account(result, "110")
    # INV-2033 is dated before the prior report date, so it is outside the window.
    assert debtors["transactions_in_window"] == 3
    assert [driver["TransactionID"] for driver in debtors["drivers"]] == ["INV-2041", "INV-2042"]
    assert (debtors["movement"], debtors["transactions_total"], debtors["unexplained"]) == (
        "12000.00", "12000.00", "0.00")

    expenses = _account(result, "500")
    assert (expenses["transactions_total"], expenses["unexplained"]) == ("11400.00", "600.00")

    # Accounts with no supplied transactions are listed, wholly unexplained.
    bank = _account(result, "100")
    assert (bank["drivers"], bank["unexplained"]) == ([], "15000.00")
    assert len(result["accounts"]) == 7
    assert result["status"] == "REVIEW"
    assert result["window"] == {"after": "2026-06-30", "through": "2026-07-31"}
    assert "transactions" in result["source_sha256"]


def test_a_covered_account_shows_nothing_unexplained(pack: Path, tmp_path: Path) -> None:
    only_debtors = tmp_path / "debtors.csv"
    only_debtors.write_text(HEADER + "".join(
        line + "\n" for line in TRANSACTIONS.read_text(encoding="utf-8").splitlines()[1:4]
    ), encoding="utf-8")
    document = json.loads((pack / "close-review-pack.json").read_text(encoding="utf-8"))
    variances = [item for item in document["exceptions"] if item["control"] == "period_variance"]
    assert len(variances) == 7  # The check below is only meaningful with every variance present.

    result = variance_drivers(pack, only_debtors)
    assert _account(result, "110")["unexplained"] == "0.00"
    assert result["status"] == "REVIEW"  # The other 6 accounts are still unexplained.


@pytest.mark.parametrize(
    ("rows", "error"),
    [
        ("Varrock Ventures Pty Ltd,110,AUD,A1,2026-07-04,,,10.00,5.00\n", ControlInputError),
        ("Varrock Ventures Pty Ltd,110,AUD,A1,04/07/2026,,,10.00,0\n", ControlInputError),
        ("Varrock Ventures Pty Ltd,110,AUD,A1,2026-07-04,,,10.00,0\n" * 2, DuplicateKeyError),
    ],
)
def test_malformed_transactions_are_refused(pack: Path, tmp_path: Path, rows: str,
                                            error: type[Exception]) -> None:
    path = tmp_path / "bad.csv"
    path.write_text(HEADER + rows, encoding="utf-8")
    with pytest.raises(error):
        variance_drivers(pack, path)


def test_a_tampered_pack_is_refused(pack: Path) -> None:
    summary = pack / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8") + "\nEdited.\n", encoding="utf-8")
    with pytest.raises(ControlInputError):
        variance_drivers(pack, TRANSACTIONS)


def test_the_cli_writes_both_files_and_guards_formula_text(pack: Path, tmp_path: Path) -> None:
    transactions = tmp_path / "formula.csv"
    transactions.write_text(
        HEADER + "Varrock Ventures Pty Ltd,110,AUD,A1,2026-07-04,=HYPERLINK(1),@cmd,12000.00,0\n",
        encoding="utf-8")
    output = tmp_path / "drivers"
    assert main(["drivers", "--pack-dir", str(pack), "--transactions", str(transactions),
                 "--output", str(output)]) == 2
    with (output / "variance-drivers.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    (debtors,) = [row for row in rows if row["AccountID"] == "110"]
    assert (debtors["Rank"], debtors["Reference"], debtors["Description"]) == ("1", "'=HYPERLINK(1)", "'@cmd")
    assert json.loads((output / "variance-drivers.json").read_text(encoding="utf-8"))["status"] == "REVIEW"
    # A second run into the same directory is refused rather than overwriting evidence.
    assert main(["drivers", "--pack-dir", str(pack), "--transactions", str(transactions),
                 "--output", str(output)]) == 1
    assert main(["drivers", "--pack-dir", str(pack), "--transactions", str(transactions),
                 "--top", "0", "--output", str(tmp_path / "other")]) == 1
