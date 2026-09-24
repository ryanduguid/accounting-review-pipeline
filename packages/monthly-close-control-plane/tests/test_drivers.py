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
    result = variance_drivers(pack, TRANSACTIONS, currency="AUD", top=2)

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
    assert (result["financial_year_reset"], result["currency"]) == (True, "AUD")
    assert "transactions" in result["source_sha256"]


def test_a_covered_account_shows_nothing_unexplained(pack: Path, tmp_path: Path) -> None:
    only_debtors = tmp_path / "debtors.csv"
    only_debtors.write_text(HEADER + "".join(
        line + "\n" for line in TRANSACTIONS.read_text(encoding="utf-8").splitlines()[1:4]
    ), encoding="utf-8")
    document = json.loads((pack / "close-review-pack.json").read_text(encoding="utf-8"))
    variances = [item for item in document["exceptions"] if item["control"] == "period_variance"]
    assert len(variances) == 7  # The check below is only meaningful with every variance present.

    result = variance_drivers(pack, only_debtors, currency="AUD")
    assert _account(result, "110")["unexplained"] == "0.00"
    assert result["status"] == "REVIEW"  # The other 6 accounts are still unexplained.


@pytest.mark.parametrize(
    ("rows", "error"),
    [
        ("Varrock Ventures Pty Ltd,110,AUD,A1,2026-07-04,,,10.00,5.00\n", ControlInputError),
        ("Varrock Ventures Pty Ltd,110,AUD,A1,04/07/2026,,,10.00,0\n", ControlInputError),
        ("Varrock Ventures Pty Ltd,110,AUD,A1,2026-07-04,,,10.00,0\n" * 2, DuplicateKeyError),
        ("Varrock Ventures Pty Ltd,110,NZD,A1,2026-07-04,,,10.00,0\n", ControlInputError),
    ],
)
def test_malformed_transactions_are_refused(pack: Path, tmp_path: Path, rows: str,
                                            error: type[Exception]) -> None:
    path = tmp_path / "bad.csv"
    path.write_text(HEADER + rows, encoding="utf-8")
    with pytest.raises(error):
        variance_drivers(pack, path, currency="AUD")


def test_a_tampered_pack_is_refused(pack: Path) -> None:
    summary = pack / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8") + "\nEdited.\n", encoding="utf-8")
    with pytest.raises(ControlInputError):
        variance_drivers(pack, TRANSACTIONS, currency="AUD")


def test_the_cli_writes_both_files_and_guards_formula_text(pack: Path, tmp_path: Path) -> None:
    transactions = tmp_path / "formula.csv"
    transactions.write_text(
        HEADER + "Varrock Ventures Pty Ltd,110,AUD,A1,2026-07-04,=HYPERLINK(1),@cmd,12000.00,0\n",
        encoding="utf-8")
    output = tmp_path / "drivers"
    assert main(["drivers", "--pack-dir", str(pack), "--transactions", str(transactions), "--currency", "AUD",
                 "--output", str(output)]) == 2
    with (output / "variance-drivers.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    (debtors,) = [row for row in rows if row["AccountID"] == "110"]
    assert (debtors["Rank"], debtors["Reference"], debtors["Description"]) == ("1", "'=HYPERLINK(1)", "'@cmd")
    assert json.loads((output / "variance-drivers.json").read_text(encoding="utf-8"))["status"] == "REVIEW"
    # A second run into the same directory is refused rather than overwriting evidence.
    assert main(["drivers", "--pack-dir", str(pack), "--transactions", str(transactions), "--currency", "AUD",
                 "--output", str(output)]) == 1
    assert main(["drivers", "--pack-dir", str(pack), "--transactions", str(transactions), "--currency", "AUD",
                 "--top", "0", "--output", str(tmp_path / "other")]) == 1


def test_a_fully_covered_run_passes_only_inside_one_financial_year(tmp_path: Path) -> None:
    header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n"

    def tb(name: str, report_date: str, bank: str) -> Path:
        path = tmp_path / name
        path.write_text(header + f"{report_date},Demo,Assets,100,Bank,090,0,0,{bank},0\n"
                        f"{report_date},Demo,Equity,900,Capital,970,0,0,0,{bank}\n", encoding="utf-8")
        return path

    transactions = tmp_path / "bank.csv"
    transactions.write_text(HEADER + "Demo,100,AUD,T1,2026-08-10,,,5000.00,0\n"
                            "Demo,900,AUD,T2,2026-08-10,,,0,5000.00\n", encoding="utf-8")
    output = tmp_path / "pack"
    main(["review", "--current", str(tb("current.csv", "2026-08-31", "15000.00")),
          "--prior", str(tb("prior.csv", "2026-07-31", "10000.00")), "--output", str(output)])
    result = variance_drivers(output, transactions, currency="AUD")
    assert result["financial_year_reset"] is False
    assert [item["unexplained"] for item in result["accounts"]] == ["0.00", "0.00"]
    assert result["status"] == "PASS"

    # The same nil remainders across a 30 June reset cannot pass.
    reset_pack = tmp_path / "reset-pack"
    main(["review", "--current", str(tb("current-reset.csv", "2026-08-31", "15000.00")),
          "--prior", str(tb("prior-reset.csv", "2026-06-30", "10000.00")), "--output", str(reset_pack)])
    reset = variance_drivers(reset_pack, transactions, currency="AUD")
    assert [item["unexplained"] for item in reset["accounts"]] == ["0.00", "0.00"]
    assert (reset["financial_year_reset"], reset["status"]) == (True, "REVIEW")
