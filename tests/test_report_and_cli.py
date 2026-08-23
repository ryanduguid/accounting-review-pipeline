from __future__ import annotations

import codecs
import csv
import json
import shutil
import subprocess
from decimal import Decimal
from pathlib import Path

import pytest

from closecontrol.cli import main
from closecontrol.engine import CloseReviewPack, review_close
from closecontrol.errors import ControlInputError
from closecontrol.loader import load_canonical_tb
from closecontrol.models import ExceptionItem
from closecontrol.report import write_review_pack


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
PACK_FILES = ["close-review-pack.json", "close-summary.md", "exceptions.csv"]


def _single_exception_pack(
    *,
    digest: str = "abc",
    tenant: str = "Varrock Ventures Pty Ltd",
    account_name: str = "Operating Bank",
    difference: str = "15000.00",
    threshold: str = "10000.00",
    reconciliation_tolerance: str = "0.01",
    percentage_threshold: str = "0.10",
    percentage_change: str | None = None,
) -> CloseReviewPack:
    return CloseReviewPack(
        status="REVIEW",
        current_report_dates=("2026-07-31",),
        prior_report_dates=("2026-06-30",),
        source_hashes={"current_trial_balance": digest},
        absolute_threshold=Decimal("1000"),
        percentage_threshold=Decimal(percentage_threshold),
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
                percentage_change=None if percentage_change is None else Decimal(percentage_change),
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
            ExceptionItem(
                control="period_variance",
                status="REVIEW",
                tenant="-A1",
                account_id="@a1",
                account_code="+XFD1048576",
                account_name="-B12",
                current_value=Decimal("3000"),
                prior_value=Decimal("0"),
                difference=Decimal("3000"),
                threshold=Decimal("100"),
                percentage_change=None,
                reason="Demo only.",
                reviewer_action="Review.",
            ),
            ExceptionItem(
                control="period_variance",
                status="REVIEW",
                tenant=" \t=1+1",
                account_id="\t+SUM(A1)",
                account_code="  -42",
                account_name="'=already-safe",
                current_value=Decimal("4000"),
                prior_value=Decimal("0"),
                difference=Decimal("4000"),
                threshold=Decimal("100"),
                percentage_change=None,
                reason="Demo only.",
                reviewer_action="Review.",
                review_group="  @unsafe",
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
    # Every formula-leading text field is neutralised, including values that
    # resemble identifiers or numbers. Monetary output fields are separate and
    # remain numeric text without an apostrophe.
    assert rows[0]["tenant"] == "'=untrusted"
    assert rows[0]["account_id"] == "'@123"
    assert rows[0]["account_code"] == "'-1000"
    assert rows[0]["account_name"] == "'+unsafe"
    assert rows[0]["current_value"] == "1000.00"
    assert rows[1]["tenant"] == "'-2+3"
    assert rows[1]["account_id"] == "'@SUM(A1)"
    assert rows[1]["account_code"] == "'+1-1"
    assert rows[1]["account_name"] == "'-cmd|xyz"
    # Cell-reference-shaped values are representative formula-leading text;
    # the guard does not need to interpret or bound the remainder.
    assert rows[2]["tenant"] == "'-A1"
    assert rows[2]["account_id"] == "'@a1"
    assert rows[2]["account_code"] == "'+XFD1048576"
    assert rows[2]["account_name"] == "'-B12"
    # Leading whitespace cannot bypass the guard, and a value already prefixed
    # with an apostrophe is not double-neutralised.
    assert rows[3]["tenant"] == "' \t=1+1"
    assert rows[3]["account_id"] == "'\t+SUM(A1)"
    assert rows[3]["account_code"] == "'  -42"
    assert rows[3]["account_name"] == "'=already-safe"
    assert rows[3]["review_group"] == "'  @unsafe"


def _table_cells(row: str) -> list[str]:
    """Split a Markdown table row into cells the way a CommonMark parser does.

    A backslash consumes the character after it, so `\\|` is one literal pipe
    inside a cell and `\\\\|` is one literal backslash followed by a live
    delimiter. Counting occurrences of the substring `\\|` cannot tell those two
    apart - it scores `\\\\|` as an escaped pipe - so the row has to be scanned
    rather than counted. Leading and trailing delimiters are optional in GFM and
    produce no cell.
    """
    cells: list[str] = []
    current: list[str] = []
    index = 0
    while index < len(row):
        character = row[index]
        if character == "\\" and index + 1 < len(row):
            current.append(row[index + 1])
            index += 2
            continue
        if character == "|":
            cells.append("".join(current))
            current = []
            index += 1
            continue
        current.append(character)
        index += 1
    cells.append("".join(current))
    if cells and not cells[0].strip():
        cells = cells[1:]
    if cells and not cells[-1].strip():
        cells = cells[:-1]
    return [cell.strip() for cell in cells]


def test_markdown_table_escapes_pipes_backslashes_and_newlines(tmp_path: Path) -> None:
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
                tenant="Varrock | Ventures",
                account_id="100",
                account_code="1000",
                # A backslash immediately before a pipe: escaping the pipe alone
                # renders '\\|', which a parser reads as an escaped backslash
                # and then a live delimiter.
                account_name="Bank \\| Operating",
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
    lines = summary.splitlines()
    header_row = next(line for line in lines if line.startswith("| Status |"))
    data_row = next(line for line in lines if line.startswith("| REVIEW |"))

    # An extra cell shifts every column after it, so the reviewer reads the
    # value under the wrong heading. The row must parse to the same six cells
    # the header declares, each holding the value it was given.
    assert _table_cells(header_row) == ["Status", "Control", "Tenant", "Account", "Difference", "Reason"]
    assert _table_cells(data_row) == [
        "REVIEW",
        "period_variance",
        "Varrock | Ventures",
        "1000 / Bank \\| Operating",
        "1000.00",
        "Line one. Line two | with pipe.",
    ]


def test_a_backslash_before_a_pipe_in_a_source_csv_cannot_shift_a_summary_column(tmp_path: Path) -> None:
    # The account name is ordinary canonical-CSV text: the loader accepts it,
    # so the summary table's structure has to survive it.
    name = "Bank \\| 0.00 | Agreed to bank statement; no action."
    header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"
    current = tmp_path / "current.csv"
    prior = tmp_path / "prior.csv"
    current.write_text(
        "\n".join([
            header,
            f'2026-07-31,Varrock Ventures Pty Ltd,Assets,1770,"{name}",1770,0.00,0.00,900000.00,0.00',
            "2026-07-31,Varrock Ventures Pty Ltd,Equity,900,Retained Earnings,3000,0.00,0.00,0.00,900000.00",
        ]) + "\n",
        encoding="utf-8",
    )
    prior.write_text(
        "\n".join([
            header,
            f'2026-06-30,Varrock Ventures Pty Ltd,Assets,1770,"{name}",1770,0.00,0.00,100000.00,0.00',
            "2026-06-30,Varrock Ventures Pty Ltd,Equity,900,Retained Earnings,3000,0.00,0.00,0.00,100000.00",
        ]) + "\n",
        encoding="utf-8",
    )

    pack = review_close(
        current_path=current,
        prior_path=prior,
        absolute_threshold=Decimal("10000"),
        percentage_threshold=Decimal("0.10"),
    )
    summary = write_review_pack(pack, tmp_path / "pack")["summary"].read_text(encoding="utf-8")
    data_row = next(
        line for line in summary.splitlines() if line.startswith("| REVIEW |") and "1770" in line
    )

    cells = _table_cells(data_row)
    assert len(cells) == 6
    # The 800,000.00 movement stays under Difference; without the backslash
    # escape it lands under Reason and the account name's own text is read as
    # the difference.
    assert cells[3] == f"1770 / {name}"
    assert cells[4] == "800000.00"
    assert cells[5].startswith("YTD net balance moved beyond")


def test_a_missing_tenant_account_or_difference_renders_as_ascii(tmp_path: Path) -> None:
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
                control="period_comparison",
                status="REVIEW",
                tenant="",
                account_id="",
                account_code="",
                account_name="",
                current_value=None,
                prior_value=None,
                difference=None,
                threshold=None,
                percentage_change=None,
                reason="Demo only.",
                reviewer_action="Review.",
            ),
        ),
        acknowledgement=None,
    )

    summary = write_review_pack(pack, tmp_path / "pack")["summary"].read_text(encoding="utf-8")
    data_row = next(line for line in summary.splitlines() if line.startswith("| REVIEW |"))

    # An em dash placeholder is unencodable on a console or scheduler log whose
    # code page is not UTF-8, so the pack's own runtime text stays ASCII; only
    # source-supplied names may carry anything wider.
    assert summary.isascii()
    assert _table_cells(data_row) == ["REVIEW", "period_comparison", "n/a", "n/a", "n/a", "Demo only."]


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
    # A multi-line comment renders as a blockquote: each source line survives
    # on its own quoted line, the pipe is escaped, and the leading '#' is
    # escaped so the quoted text cannot become a document heading.
    assert "- Comment:\n  > Reviewed.\n  > \\## Forged approval \\| yes" in summary


def test_multi_line_reviewer_comment_keeps_its_line_structure(tmp_path: Path) -> None:
    source = (EXAMPLES / "review_note.json").read_text(encoding="utf-8")
    note = tmp_path / "review.json"
    payload = json.loads(source)
    payload["comment"] = "Variance driver confirmed.\nDebtors follow-up booked for next close.\nNo journal was posted."
    note.write_text(json.dumps(payload), encoding="utf-8")
    pack = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        acknowledgement_path=note,
    )

    summary = write_review_pack(pack, tmp_path / "pack")["summary"].read_text(encoding="utf-8")
    assert (
        "- Comment:\n"
        "  > Variance driver confirmed.\n"
        "  > Debtors follow-up booked for next close.\n"
        "  > No journal was posted.\n"
        "- Effect:" in summary
    )


def test_single_line_reviewer_comment_stays_inline(tmp_path: Path) -> None:
    pack = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        acknowledgement_path=EXAMPLES / "review_note.json",
    )

    summary = write_review_pack(pack, tmp_path / "pack")["summary"].read_text(encoding="utf-8")
    assert "- Comment: Reviewed fabricated demo exceptions only" in summary


@pytest.mark.parametrize("blocked", PACK_FILES)
def test_a_failed_pack_write_rolls_back_to_the_previous_run(tmp_path: Path, blocked: str) -> None:
    output = tmp_path / "pack"
    write_review_pack(_single_exception_pack(digest="aaa", difference="15000.00"), output)
    survivors = {name: (output / name).read_bytes() for name in PACK_FILES if name != blocked}
    # Stand in for the reviewer's spreadsheet holding a pack file open: the path
    # exists and can be neither written nor replaced.
    stand_in = output / blocked
    stand_in.unlink()
    stand_in.mkdir()
    (stand_in / "held-open.txt").write_text("locked", encoding="utf-8")

    with pytest.raises(OSError):
        write_review_pack(_single_exception_pack(digest="bbb", difference="85000.00"), output)

    # A run that cannot finish must leave the previous pack whole, whichever of
    # the three files blocks it. Deleting evidence this run never wrote - the
    # untouched exception detail from the last close - is worse than the mixed
    # pack the staging exists to prevent, and the CLI reports only the OSError.
    for name, content in survivors.items():
        assert (output / name).read_bytes() == content
    assert sorted(item.name for item in output.iterdir()) == PACK_FILES


def test_a_failed_write_removes_a_pack_file_that_had_no_previous_version(tmp_path: Path) -> None:
    output = tmp_path / "pack"
    write_review_pack(_single_exception_pack(digest="aaa", difference="15000.00"), output)
    # The directory holds only part of a previous pack: this file was deleted,
    # or the run that should have written it was interrupted.
    (output / "close-review-pack.json").unlink()
    survivor = (output / "close-summary.md").read_bytes()
    stand_in = output / "exceptions.csv"
    stand_in.unlink()
    stand_in.mkdir()
    (stand_in / "held-open.txt").write_text("locked", encoding="utf-8")

    with pytest.raises(OSError):
        write_review_pack(_single_exception_pack(digest="bbb", difference="85000.00"), output)

    # Rolling back only the files that had something to restore would leave this
    # run's close-review-pack.json beside the previous run's close-summary.md -
    # the mixed pack the staging exists to prevent, with nothing on the face of
    # either file to tell a reviewer they describe different trial balances.
    assert not (output / "close-review-pack.json").exists()
    assert (output / "close-summary.md").read_bytes() == survivor
    assert sorted(item.name for item in output.iterdir()) == ["close-summary.md", "exceptions.csv"]


def test_a_staging_write_that_dies_part_way_leaves_no_orphan(monkeypatch, tmp_path: Path) -> None:
    output = tmp_path / "pack"
    real_write_text = Path.write_text
    calls = {"count": 0}

    def flaky(self: Path, data: str, *args, **kwargs):
        calls["count"] += 1
        if calls["count"] == 2:
            real_write_text(self, data[:40], *args, **kwargs)
            raise OSError(28, "No space left on device")
        return real_write_text(self, data, *args, **kwargs)

    monkeypatch.setattr(Path, "write_text", flaky)

    with pytest.raises(OSError):
        write_review_pack(_single_exception_pack(), output)

    # A staged file the run could not finish writing must go with the rest, not
    # sit in the output directory as a truncated fragment of a pack.
    assert list(output.iterdir()) == []


def test_a_staged_file_from_another_run_is_not_reused(tmp_path: Path) -> None:
    output = tmp_path / "pack"
    output.mkdir(parents=True)
    # A concurrent run staging into the same directory holds this file. Staging
    # under a fixed name overwrites it and then moves that run's content into
    # place beside this run's other two files.
    decoy = output / "close-review-pack.json.partial"
    decoy.write_text("another run's staged pack", encoding="utf-8")

    write_review_pack(_single_exception_pack(digest="aaa"), output)

    assert decoy.read_text(encoding="utf-8") == "another run's staged pack"
    assert "aaa" in (output / "close-review-pack.json").read_text(encoding="utf-8")


def test_a_percentage_finer_than_a_hundredth_of_a_percent_survives_into_every_pack_file(tmp_path: Path) -> None:
    pack = _single_exception_pack(percentage_threshold="0.000004", percentage_change="0.000004")

    outputs = write_review_pack(pack, tmp_path / "pack")

    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    assert payload["thresholds"]["percentage_variance"] == "0.0004%"
    assert payload["exceptions"][0]["percentage_change"] == "0.0004%"
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        row = next(csv.DictReader(source))
    assert row["percentage_change"] == "0.0004%"
    summary = outputs["summary"].read_text(encoding="utf-8")
    assert "- Material variance thresholds: $1000.00 and 0.0004%" in summary


def test_a_percentage_carrying_full_division_precision_still_renders_short(tmp_path: Path) -> None:
    # percentage_change comes from a Decimal division, so it can carry the full
    # context precision; the scale must not follow it to 28 places.
    ratio = Decimal(1) / Decimal(3)
    pack = _single_exception_pack(percentage_change=str(ratio))

    outputs = write_review_pack(pack, tmp_path / "pack")

    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    assert payload["exceptions"][0]["percentage_change"] == "33.33%"


def test_a_prior_zero_percentage_renders_as_an_explicit_sentinel_not_a_blank(tmp_path: Path) -> None:
    # A period_variance exception against a nil prior balance carries
    # percentage_change=None. A blank cell in the CSV or JSON reads as
    # "no change", so the pack must state the condition in text that cannot
    # parse as a number.
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
                tenant="Varrock Ventures Pty Ltd",
                account_id="777",
                account_code="1770",
                account_name="New Clearing",
                current_value=Decimal("900000.00"),
                prior_value=Decimal("0.00"),
                difference=Decimal("900000.00"),
                threshold=Decimal("10000"),
                percentage_change=None,
                reason="Demo only.",
                reviewer_action="Review.",
            ),
        ),
        acknowledgement=None,
    )

    outputs = write_review_pack(pack, tmp_path / "pack")

    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    assert payload["exceptions"][0]["percentage_change"] == "n/a (prior period zero)"
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        row = next(csv.DictReader(source))
    assert row["percentage_change"] == "n/a (prior period zero)"
    # The sentinel must never be readable as a number by a downstream consumer.
    with pytest.raises(Exception):
        Decimal(row["percentage_change"])


def test_controls_without_a_percentage_still_render_an_empty_percentage_cell(tmp_path: Path) -> None:
    # The sentinel names one specific condition. A control that never computes
    # a percentage (reconciliation here, prior_value holds the subledger
    # balance) keeps its empty cell; writing the sentinel there would claim a
    # prior-period condition the control never evaluated.
    pack = _single_exception_pack()

    outputs = write_review_pack(pack, tmp_path / "pack")

    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    assert payload["exceptions"][0]["percentage_change"] == ""
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        row = next(csv.DictReader(source))
    assert row["percentage_change"] == ""


def test_a_nil_prior_variance_from_the_engine_reaches_the_pack_with_the_sentinel(tmp_path: Path) -> None:
    # End to end: the engine leaves percentage_change as None for a nil prior
    # balance and the writers must carry the explicit sentinel, not a blank.
    header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"
    current = tmp_path / "current.csv"
    prior = tmp_path / "prior.csv"
    prior.write_text(
        "\n".join([
            header,
            "2026-06-30,Varrock Ventures Pty Ltd,Assets,777,New Clearing,1770,0.00,0.00,0.00,0.00",
            "2026-06-30,Varrock Ventures Pty Ltd,Equity,900,Retained Earnings,3000,0.00,0.00,0.00,0.00",
        ]) + "\n",
        encoding="utf-8",
    )
    current.write_text(
        "\n".join([
            header,
            "2026-07-31,Varrock Ventures Pty Ltd,Assets,777,New Clearing,1770,900000.00,0.00,900000.00,0.00",
            "2026-07-31,Varrock Ventures Pty Ltd,Equity,900,Retained Earnings,3000,0.00,900000.00,0.00,900000.00",
        ]) + "\n",
        encoding="utf-8",
    )
    pack = review_close(
        current_path=current,
        prior_path=prior,
        absolute_threshold=Decimal("10000"),
    )

    outputs = write_review_pack(pack, tmp_path / "pack")

    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    variance = next(
        item for item in payload["exceptions"]
        if item["control"] == "period_variance" and item["account_id"] == "777"
    )
    assert variance["percentage_change"] == "n/a (prior period zero)"
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        rows = list(csv.DictReader(source))
    csv_variance = next(
        row for row in rows
        if row["control"] == "period_variance" and row["account_id"] == "777"
    )
    assert csv_variance["percentage_change"] == "n/a (prior period zero)"
    # The markdown table has no percentage column; its Reason cell already
    # states that the percentage threshold was not tested, so the summary
    # carries the same fact in prose.
    summary = outputs["summary"].read_text(encoding="utf-8")
    assert "so the percentage threshold was not tested" in summary


def test_amounts_that_are_not_decimals_still_render(tmp_path: Path) -> None:
    # CloseReviewPack and ExceptionItem are frozen dataclasses with no runtime
    # type enforcement, so a library caller can hand the writer an int or float.
    pack = CloseReviewPack(
        status="REVIEW",
        current_report_dates=("2026-07-31",),
        prior_report_dates=("2026-06-30",),
        source_hashes={"current_trial_balance": "abc"},
        absolute_threshold=1000,
        percentage_threshold=0.10,
        reconciliation_tolerance=0.01,
        exceptions=(
            ExceptionItem(
                control="period_variance",
                status="REVIEW",
                tenant="Varrock Ventures Pty Ltd",
                account_id="100",
                account_code="1000",
                account_name="Operating Bank",
                current_value=1000,
                prior_value=0,
                difference=1000.5,
                threshold=100,
                percentage_change=0.25,
                reason="Demo only.",
                reviewer_action="Review.",
            ),
        ),
        acknowledgement=None,
    )

    payload = json.loads(write_review_pack(pack, tmp_path / "pack")["json"].read_text(encoding="utf-8"))

    assert payload["thresholds"]["absolute_variance"] == "1000.00"
    assert payload["thresholds"]["percentage_variance"] == "10.00%"
    assert payload["exceptions"][0]["difference"] == "1000.50"
    assert payload["exceptions"][0]["percentage_change"] == "25.00%"


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


def test_exceptions_csv_and_json_carry_the_review_group_column(tmp_path: Path) -> None:
    # The --mapping file's ReviewGroup is the grouping a reviewer filters by,
    # so the pack must carry it on every exception; blank when the run had no
    # mapping to consult.
    with_mapping = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        mapping_path=EXAMPLES / "account_mapping.csv",
        absolute_threshold=Decimal("10000"),
    )
    outputs = write_review_pack(with_mapping, tmp_path / "mapped")
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        reader = csv.DictReader(source)
        assert "review_group" in (reader.fieldnames or [])
        rows = list(reader)
    by_account = {(row["control"], row["account_id"]): row["review_group"] for row in rows}
    assert by_account[("period_variance", "100")] == "Cash and cash equivalents"
    assert by_account[("period_variance", "110")] == "Receivables"
    # Account 500 has no mapping row, so its group stays blank.
    assert by_account[("account_mapping", "500")] == ""
    payload = json.loads(outputs["json"].read_text(encoding="utf-8"))
    json_variance = next(
        item for item in payload["exceptions"]
        if item["control"] == "period_variance" and item["account_id"] == "100"
    )
    assert json_variance["review_group"] == "Cash and cash equivalents"

    without_mapping = review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        absolute_threshold=Decimal("10000"),
    )
    outputs = write_review_pack(without_mapping, tmp_path / "unmapped")
    with outputs["exceptions"].open(encoding="utf-8-sig", newline="") as source:
        unmapped_rows = list(csv.DictReader(source))
    assert unmapped_rows
    assert all(row["review_group"] == "" for row in unmapped_rows)


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


def test_workbench_writes_the_existing_review_pack_and_hands_off_to_the_reviewer(
    capsys, tmp_path: Path
) -> None:
    """A missing workbench façade would make the command unrecognised.

    The workbench must keep the existing review engine and pack writer as the
    one source of truth: a reviewer receives the same three artefacts and an
    explicit reminder that the pack is not an approval.
    """
    output = tmp_path / "workbench-pack"

    exit_code = main([
        "workbench",
        "--current", str(EXAMPLES / "current_trial_balance.csv"),
        "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
        "--mapping", str(EXAMPLES / "account_mapping.csv"),
        "--subledger", str(EXAMPLES / "subledger_balances.csv"),
        "--review-note", str(EXAMPLES / "review_note.json"),
        "--absolute-threshold", "10000",
        "--percentage-threshold", "0.10",
        "--output", str(output),
    ])

    assert exit_code == 2
    assert {path.name for path in output.iterdir()} == set(PACK_FILES)
    payload = json.loads((output / "close-review-pack.json").read_text(encoding="utf-8"))
    assert payload["overall_status"] == "REVIEW"
    stdout = capsys.readouterr().out
    assert "close-control workbench: REVIEW; 8 exception(s)" in stdout
    assert "Review close-summary.md, exceptions.csv, and close-review-pack.json." in stdout
    assert "does not approve or close a period" in stdout


def test_workbench_refuses_a_source_that_would_be_replaced_by_the_pack(
    tmp_path: Path
) -> None:
    """Removing the shared collision preflight would destroy a supplied source."""
    work = tmp_path / "workbench-client"
    work.mkdir()
    shutil.copy(EXAMPLES / "current_trial_balance.csv", work / "current.csv")
    shutil.copy(EXAMPLES / "prior_trial_balance.csv", work / "prior.csv")
    subledger = work / "exceptions.csv"
    subledger.write_text(
        "Tenant,AccountID,SubledgerBalance\nVarrock Ventures Pty Ltd,110,85000.00\n",
        encoding="utf-8",
    )
    before = subledger.read_bytes()

    code = main([
        "workbench",
        "--current", str(work / "current.csv"),
        "--prior", str(work / "prior.csv"),
        "--subledger", str(subledger),
        "--output", str(work),
    ])

    assert code == 1
    assert subledger.read_bytes() == before
    assert not (work / "close-review-pack.json").exists()


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


@pytest.mark.parametrize(
    "candidate",
    [f"{directory}/{name}" for directory in ("examples", "schemas") for name in PACK_FILES],
)
def test_gitignore_blocks_a_generated_pack_wherever_output_points(candidate: str) -> None:
    if shutil.which("git") is None or not (ROOT / ".git").exists():
        pytest.skip("not a git checkout")

    result = subprocess.run(["git", "check-ignore", "-q", candidate], cwd=ROOT)

    # examples/ and schemas/ re-include their CSVs so the fabricated fixtures
    # stay committable, which left exceptions.csv committable with them.
    assert result.returncode == 0, f"{candidate} is not ignored"


def test_gitignore_still_admits_the_fabricated_fixtures() -> None:
    if shutil.which("git") is None or not (ROOT / ".git").exists():
        pytest.skip("not a git checkout")

    for fixture in ("examples/current_trial_balance.csv", "schemas/canonical_trial_balance.csv"):
        assert subprocess.run(["git", "check-ignore", "-q", fixture], cwd=ROOT).returncode == 1, fixture


def test_cli_help_still_exits_zero(capsys) -> None:
    with pytest.raises(SystemExit) as excinfo:
        main(["--help"])

    assert excinfo.value.code == 0
    capsys.readouterr()


def test_a_source_file_sharing_a_pack_name_is_refused_not_destroyed(tmp_path: Path) -> None:
    """--subledger <dir>/exceptions.csv --output <dir> used to overwrite the
    source with the generated exceptions file and still report success, so the
    source_sha256 in the pack described a file that no longer existed."""
    work = tmp_path / "client"
    work.mkdir()
    shutil.copy(EXAMPLES / "current_trial_balance.csv", work / "current.csv")
    shutil.copy(EXAMPLES / "prior_trial_balance.csv", work / "prior.csv")
    subledger = work / "exceptions.csv"
    subledger.write_text(
        "Tenant,AccountID,SubledgerBalance\nVarrock Ventures Pty Ltd,110,85000.00\n",
        encoding="utf-8",
    )
    before = subledger.read_bytes()

    code = main([
        "review",
        "--current", str(work / "current.csv"),
        "--prior", str(work / "prior.csv"),
        "--subledger", str(subledger),
        "--output", str(work),
    ])

    assert code == 1
    assert subledger.read_bytes() == before
    assert not (work / "close-review-pack.json").exists()


def test_a_review_note_surrogate_is_refused_instead_of_crashing(tmp_path: Path) -> None:
    """A lone surrogate survived the control-character gate (category Cs), then
    UnicodeEncodeError - a ValueError, not an OSError - escaped both the staging
    cleanup and the CLI handler, leaving a .partial holding the whole pack."""
    work = tmp_path / "client"
    work.mkdir()
    note = tmp_path / "note.json"
    # written as bytes so the file holds the six-character escape; json.loads
    # is what turns it into a lone surrogate.
    note.write_bytes(
        b'{"reviewer_initials":"RD","reviewed_on":"2026-08-08",'
        rb'"comment":"Reviewed \ud800 demo."}'
    )

    code = main([
        "review",
        "--current", str(EXAMPLES / "current_trial_balance.csv"),
        "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
        "--review-note", str(note),
        "--output", str(work),
    ])

    assert code == 1
    assert list(work.glob("*.partial")) == []
    assert list(work.glob("close-*")) == []


def test_a_blank_line_does_not_shift_the_reported_row_number(tmp_path: Path) -> None:
    """DictReader silently skips blank rows, so an enumerate counter drifts
    below the physical file line from the first blank line onwards and every
    later error names a row the reader cannot find."""
    csv_path = tmp_path / "current.csv"
    csv_path.write_text(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,"
        "Debit,Credit,YTDDebit,YTDCredit\n"
        "2026-07-31,Varrock,Assets,100,Operating Bank,1000,0.00,0.00,1000.00,0.00\n"
        "\n"
        "2026-07-31,Varrock,Assets,110,,1100,0.00,0.00,500.00,0.00\n",
        encoding="utf-8",
        newline="",
    )

    with pytest.raises(ControlInputError) as caught:
        load_canonical_tb(csv_path)

    # The offending record is physically on line 4: header, row, blank, row.
    assert "row 4" in str(caught.value)
    assert "row 3" not in str(caught.value)
