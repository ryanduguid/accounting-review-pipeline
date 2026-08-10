from __future__ import annotations

import codecs
import csv
import json
from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.cli import main
from closecontrol.engine import CloseReviewPack, review_close
from closecontrol.models import ExceptionItem
from closecontrol.report import write_review_pack


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
PACK_FILES = ["close-review-pack.json", "close-summary.md", "exceptions.csv"]


def _single_exception_pack(
    *,
    digest: str = "abc",
    tenant: str = "Acme Demo Pty Ltd",
    account_name: str = "Operating Bank",
    difference: str = "15000.00",
    threshold: str = "10000.00",
    reconciliation_tolerance: str = "0.01",
) -> CloseReviewPack:
    return CloseReviewPack(
        status="REVIEW",
        current_report_dates=("2026-07-31",),
        prior_report_dates=("2026-06-30",),
        source_hashes={"current_trial_balance": digest},
        absolute_threshold=Decimal("1000"),
        percentage_threshold=Decimal("0.10"),
        reconciliation_tolerance=Decimal(reconciliation_tolerance),
        exceptions=(
            ExceptionItem(
                control="subledger_reconciliation",
                status="REVIEW",
                tenant=tenant,
                account_id="100",
                account_code="1000",
                account_name=account_name,
                current_value=Decimal("1000"),
                prior_value=Decimal("0"),
                difference=Decimal(difference),
                threshold=Decimal(threshold),
                percentage_change=None,
                reason="Demo only.",
                reviewer_action="Review.",
            ),
        ),
        acknowledgement=None,
    )


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
            ExceptionItem(
                control="period_variance",
                status="REVIEW",
                tenant="-2+3",
                account_id="@SUM(A1)",
                account_code="+1-1",
                account_name="-cmd|xyz",
                current_value=Decimal("2000"),
                prior_value=Decimal("0"),
                difference=Decimal("2000"),
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
    # A completed pack is exactly the three named files; nothing staged survives.
    assert sorted(item.name for item in (tmp_path / "one").iterdir()) == PACK_FILES
    with first["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        rows = list(csv.DictReader(source))
    # '=' is always neutralised. '+', '-' and '@' are neutralised only when the
    # remainder could be read as a formula, so plain identifiers such as the
    # account code '-1000' and the ID '@123' stay joinable in Excel and Power BI.
    assert rows[0]["tenant"] == "'=untrusted"
    assert rows[0]["account_id"] == "@123"
    assert rows[0]["account_code"] == "-1000"
    assert rows[0]["account_name"] == "+unsafe"
    assert rows[1]["tenant"] == "'-2+3"
    assert rows[1]["account_id"] == "'@SUM(A1)"
    assert rows[1]["account_code"] == "'+1-1"
    assert rows[1]["account_name"] == "'-cmd|xyz"


def test_markdown_table_escapes_pipes_and_newlines(tmp_path: Path) -> None:
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
                tenant="Acme | Demo",
                account_id="100",
                account_code="1000",
                account_name="Bank | Operating",
                current_value=Decimal("1000"),
                prior_value=Decimal("0"),
                difference=Decimal("1000"),
                threshold=Decimal("100"),
                percentage_change=None,
                reason="Line one.\nLine two | with pipe.",
                reviewer_action="Review.",
            ),
        ),
        acknowledgement=None,
    )

    outputs = write_review_pack(pack, tmp_path / "pack")
    summary = outputs["summary"].read_text(encoding="utf-8")
    data_row = next(line for line in summary.splitlines() if line.startswith("| REVIEW |"))

    # Unescaped pipes would split the row into extra columns; the table row
    # must keep exactly six cells with every embedded pipe escaped.
    assert data_row.count("|") - data_row.count("\\|") == 7
    assert "Acme \\| Demo" in data_row
    assert "Bank \\| Operating" in data_row
    assert "Line one. Line two \\| with pipe." in data_row


def test_acknowledgement_cannot_inject_markdown_structure(tmp_path: Path) -> None:
    source = (EXAMPLES / "review_note.json").read_text(encoding="utf-8")
    note = tmp_path / "review.json"
    payload = json.loads(source)
    payload["comment"] = "Reviewed.\n## Forged approval | yes"
    note.write_text(json.dumps(payload), encoding="utf-8")
    pack = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        acknowledgement_path=note,
    )

    summary = write_review_pack(pack, tmp_path / "pack")["summary"].read_text(encoding="utf-8")
    assert "\n## Forged approval" not in summary
    assert "Reviewed. ## Forged approval \\| yes" in summary


def test_a_failed_pack_write_leaves_no_mixed_run_behind(tmp_path: Path) -> None:
    output = tmp_path / "pack"
    write_review_pack(_single_exception_pack(digest="aaa", difference="15000.00"), output)
    # Stand in for the reviewer's spreadsheet holding exceptions.csv open: the
    # path exists and can be neither written nor replaced.
    stand_in = output / "exceptions.csv"
    stand_in.unlink()
    stand_in.mkdir()
    (stand_in / "held-open.txt").write_text("locked", encoding="utf-8")

    with pytest.raises(OSError):
        write_review_pack(_single_exception_pack(digest="bbb", difference="85000.00"), output)

    # The JSON and the summary must not survive describing a trial balance the
    # exception detail beside them knows nothing about.
    assert sorted(item.name for item in output.iterdir()) == ["exceptions.csv"]


def test_sub_cent_amounts_survive_into_every_pack_file(tmp_path: Path) -> None:
    pack = _single_exception_pack(
        difference="0.0040", threshold="0.001", reconciliation_tolerance="0.001"
    )

    outputs = write_review_pack(pack, tmp_path / "pack")

    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    assert payload["exceptions"][0]["difference"] == "0.0040"
    assert payload["exceptions"][0]["threshold"] == "0.001"
    assert payload["thresholds"]["reconciliation_tolerance"] == "0.001"
    # Whole-dollar figures keep their familiar two places.
    assert payload["thresholds"]["absolute_variance"] == "1000.00"
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        row = next(csv.DictReader(source))
    assert row["difference"] == "0.0040"
    assert row["threshold"] == "0.001"
    summary = outputs["summary"].read_text(encoding="utf-8")
    assert "- Reconciliation tolerance: $0.001" in summary
    assert "- Material variance thresholds: $1000.00 and 10.00%" in summary
    assert "| 0.0040 |" in summary


def test_exceptions_csv_carries_a_byte_order_mark_for_spreadsheet_readers(tmp_path: Path) -> None:
    pack = _single_exception_pack(tenant="S\u00f6dra Pty Ltd", account_name="Kaff\u00e9 Konto")

    outputs = write_review_pack(pack, tmp_path / "pack")

    raw = outputs["exceptions"].read_bytes()
    # Without the mark a spreadsheet falling back to the Windows ANSI code page
    # renders these names as mojibake and they stop joining to the source export.
    assert raw.startswith(codecs.BOM_UTF8)
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        row = next(csv.DictReader(source))
    assert row["tenant"] == "S\u00f6dra Pty Ltd"
    assert row["account_name"] == "Kaff\u00e9 Konto"


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


def test_cli_reports_an_unusable_output_path_instead_of_crashing(capsys, tmp_path: Path) -> None:
    occupied = tmp_path / "notadir.txt"
    occupied.write_text("this is a file, not a directory\n", encoding="utf-8")

    exit_code = main([
        "review",
        "--current", str(EXAMPLES / "current_trial_balance.csv"),
        "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
        "--output", str(occupied),
    ])

    # A caller reading stderr for the tool's prefix must see a message, not a
    # Python traceback.
    assert exit_code == 1
    assert capsys.readouterr().err.startswith("close-control: output error:")


def test_cli_returns_one_for_usage_errors(capsys, tmp_path: Path) -> None:
    # A typo'd flag must exit 1 (invalid command configuration), never 2,
    # which the exit contract reserves for a pack needing review.
    assert main(["review", "--no-such-flag"]) == 1
    # A missing required argument is the same contract.
    assert main(["review", "--current", str(EXAMPLES / "current_trial_balance.csv")]) == 1
    # An unknown subcommand as well.
    assert main(["frobnicate"]) == 1
    capsys.readouterr()


def test_cli_help_still_exits_zero(capsys) -> None:
    with pytest.raises(SystemExit) as excinfo:
        main(["--help"])

    assert excinfo.value.code == 0
    capsys.readouterr()
