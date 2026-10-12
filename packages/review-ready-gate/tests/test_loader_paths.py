"""Loader refusals the fixture packs never reach: each names the defect it found."""

from __future__ import annotations

import json
import re
from decimal import Decimal
from pathlib import Path

import pytest
from reviewready import loader
from reviewready.errors import (
    DateMismatchError,
    DuplicateKeyError,
    GateInputError,
    SchemaError,
)
from reviewready.loader import (
    SourceSnapshot,
    load_activity_statement,
    load_bank_rec,
    load_canonical_tb,
    load_gst_control,
    load_open_items,
    load_prior_findings,
    load_reviewer_acknowledgement,
    load_self_review,
    load_tie_out_matrix,
)
from tests.support import EXAMPLES

OPEN_HEADER = "ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n"
TIE_HEADER = "StatementLine,StatementAmount,WorkpaperRef,SourceFile,Status\n"


def _snapshot(tmp_path: Path, text: str, name: str = "input.csv") -> SourceSnapshot:
    path = tmp_path / name
    path.write_text(text, encoding="utf-8")
    return SourceSnapshot.capture(path, label="test file")


def test_capture_names_a_missing_file_and_an_unreadable_one(tmp_path: Path) -> None:
    with pytest.raises(GateInputError, match="test file does not exist"):
        SourceSnapshot.capture(tmp_path / "absent.csv", label="test file")
    with pytest.raises(GateInputError, match="test file could not be read"):
        SourceSnapshot.capture(tmp_path, label="test file")


@pytest.mark.parametrize(
    "text,match",
    [
        ("", "CSV has no header row"),
        ("Label,Amount,Amount\n1A,1.00,1.00\n", r"duplicate column heading\(s\): Amount\."),
        ("Label\n1A\n", r"canonical schema mismatch \(missing Amount\)"),
        ("Label,Amount,Note\n1A,1.00,x\n", r"canonical schema mismatch \(unexpected Note\)"),
    ],
)
def test_header_defects_are_named(tmp_path: Path, text: str, match: str) -> None:
    with pytest.raises(SchemaError, match=match):
        load_activity_statement(_snapshot(tmp_path, text))


def _tb_text() -> str:
    return (EXAMPLES / "bas-ready" / "trial_balance.csv").read_text(encoding="utf-8")


def test_trial_balance_with_no_rows_is_refused(tmp_path: Path) -> None:
    header = _tb_text().splitlines()[0] + "\n"
    with pytest.raises(SchemaError, match="no trial-balance rows were supplied"):
        load_canonical_tb(_snapshot(tmp_path, header))


def test_trial_balance_with_two_report_dates_is_a_date_mismatch(tmp_path: Path) -> None:
    lines = _tb_text().splitlines()
    lines[2] = lines[2].replace("2026-03-31", "2026-03-30", 1)
    with pytest.raises(DateMismatchError, match="exactly one ReportDate"):
        load_canonical_tb(_snapshot(tmp_path, "\n".join(lines) + "\n"))


@pytest.mark.parametrize(
    "load,text,error,match",
    [
        (
            load_open_items,
            OPEN_HEADER
            + "OI-1,EXPLAIN,AB,2026-04-05,OPEN,First,\n"
            + "OI-1,EXPLAIN,AB,2026-04-05,OPEN,Second,\n",
            DuplicateKeyError,
            "duplicate ItemID 'OI-1'",
        ),
        (
            load_open_items,
            OPEN_HEADER + "OI-1,URGENT,AB,2026-04-05,OPEN,First,\n",
            SchemaError,
            "row 2 Severity must be BLOCKING, EXPLAIN, or TRIVIAL",
        ),
        (
            load_open_items,
            OPEN_HEADER + "OI-1,EXPLAIN,AB,2026-04-05,DONE,First,\n",
            SchemaError,
            "row 2 Status must be OPEN or CLEARED",
        ),
        (
            load_activity_statement,
            "Label,Amount\n1A,1.00\n1A,2.00\n",
            DuplicateKeyError,
            "duplicate Label '1A'",
        ),
        (
            load_activity_statement,
            "Label,Amount\n",
            SchemaError,
            "no activity-statement rows were supplied",
        ),
        (
            load_gst_control,
            "Date,AccountID,AccountName,Debit,Credit,Description\n",
            SchemaError,
            "no GST-control rows were supplied",
        ),
        (
            load_bank_rec,
            "AccountID,StatementBalance,GLBalance\n100,1.00,1.00\n100,2.00,2.00\n",
            DuplicateKeyError,
            "duplicate AccountID '100'",
        ),
        (
            load_bank_rec,
            "AccountID,StatementBalance,GLBalance\n",
            SchemaError,
            "no bank-rec rows were supplied",
        ),
        (
            load_tie_out_matrix,
            TIE_HEADER + "Cash,1.00,WP-1,tb.csv,TIED\nCash,1.00,WP-1,tb.csv,TIED\n",
            DuplicateKeyError,
            "duplicate StatementLine 'Cash'",
        ),
        (
            load_tie_out_matrix,
            TIE_HEADER + "Cash,1.00,WP-1,tb.csv,AGREED\n",
            SchemaError,
            "row 2 Status must be TIED, EXCEPTION, or UNSUPPORTED",
        ),
        (
            load_tie_out_matrix,
            TIE_HEADER,
            SchemaError,
            "no tie-out rows were supplied",
        ),
        (
            load_prior_findings,
            "FindingCode,Slot,Status\nMISSING_ARTEFACT,bank_rec,CLOSED\n",
            SchemaError,
            "row 2 Status must be OPEN or CLEARED",
        ),
        (
            load_prior_findings,
            "FindingCode,Slot,Status\nMISSING_ARTEFACT,bank_rec,OPEN\n"
            "MISSING_ARTEFACT,bank_rec,CLEARED\n",
            DuplicateKeyError,
            "duplicate prior finding 'MISSING_ARTEFACT' / 'bank_rec'",
        ),
    ],
)
def test_row_level_refusals_name_the_row_or_key(
    tmp_path: Path, load, text: str, error: type[Exception], match: str
) -> None:
    with pytest.raises(error, match=match):
        load(_snapshot(tmp_path, text))


def _self_review() -> dict:
    return json.loads((EXAMPLES / "bas-ready" / "self_review.json").read_text(encoding="utf-8"))


@pytest.mark.parametrize(
    "change,match",
    [
        (lambda doc: "{", "self-review is not valid JSON"),
        (lambda doc: [doc], "must contain exactly preparer_initials"),
        (lambda doc: {**doc, "preparer_initials": "A" * 13}, "at most 12 characters"),
        (
            lambda doc: {**doc, "preparer_initials": "A​B"},
            "preparer_initials contains a control or formatting character",
        ),
        (lambda doc: {**doc, "prepared_on": 20260410}, "prepared_on must be an ISO date string"),
        (lambda doc: {**doc, "period_end": None}, "period_end must be an ISO date string"),
        (
            lambda doc: {**doc, "prepared_on": "2026-03-30"},
            "prepared_on 2026-03-30 is earlier than period_end 2026-03-31",
        ),
        (
            lambda doc: {**doc, "assertions": {"pack_complete": True}},
            "assertions must contain exactly pack_complete",
        ),
        (
            lambda doc: {**doc, "assertions": {**doc["assertions"], "self_reviewed": 1}},
            "assertions.self_reviewed must be a JSON boolean",
        ),
    ],
)
def test_self_review_refusals(tmp_path: Path, change, match: str) -> None:
    payload = change(_self_review())
    text = payload if isinstance(payload, str) else json.dumps(payload)
    with pytest.raises(SchemaError, match=match):
        load_self_review(_snapshot(tmp_path, text, "self_review.json"), expected_profile="bas")


@pytest.mark.parametrize(
    "change,match",
    [
        (lambda doc: "{", "review note is not valid JSON"),
        (lambda doc: [doc], "review note must contain exactly reviewer_initials"),
        (lambda doc: {**doc, "reviewer_initials": " "}, "reviewer_initials must be a non-empty"),
        (lambda doc: {**doc, "comment": "  "}, "comment must be a non-empty string"),
        (lambda doc: {**doc, "reviewed_on": ["2026-04-12"]}, "reviewed_on must be an ISO date string"),
    ],
)
def test_review_note_refusals(tmp_path: Path, change, match: str) -> None:
    note = json.loads((EXAMPLES / "review_note.json").read_text(encoding="utf-8"))
    payload = change(note)
    path = tmp_path / "review_note.json"
    path.write_text(payload if isinstance(payload, str) else json.dumps(payload), encoding="utf-8")
    with pytest.raises(SchemaError, match=match):
        load_reviewer_acknowledgement(path)


def test_money_pattern_admits_only_text_decimal_can_convert() -> None:
    """parse_money keeps an InvalidOperation and a non-finite guard behind its pattern.

    Both are marked unreachable for coverage; this pins why. Every character the
    pattern's digit class admits converts to a finite Decimal, and the pattern
    admits no exponent, NaN or Infinity spelling.
    """
    decimal_digit = re.compile(r"\d")
    digits = [chr(code) for code in range(0x110000) if decimal_digit.fullmatch(chr(code))]
    assert all(Decimal(digit).is_finite() for digit in digits)  # nosec B101: pytest outcome assertion
    for spelling in ("NaN", "Infinity", "-Inf", "1E5", "sNaN"):
        assert not loader._ACCOUNTING_NUMBER.fullmatch(spelling)  # nosec B101: pytest outcome assertion
