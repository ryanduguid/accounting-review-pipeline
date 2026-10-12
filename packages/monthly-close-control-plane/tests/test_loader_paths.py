"""Loader refusals for unreadable files, schema drift, duplicates and review-note fields."""
from __future__ import annotations

import json
from pathlib import Path

import pytest
from closecontrol.errors import ControlInputError, DuplicateKeyError, SchemaError
from closecontrol.loader import (
    CANONICAL_COLUMNS,
    SourceSnapshot,
    load_canonical_tb,
    load_mapping,
    load_reviewer_acknowledgement,
    load_subledger,
)

ROOT = Path(__file__).resolve().parents[1]
CURRENT = ROOT / "examples" / "current_trial_balance.csv"


def test_a_directory_in_place_of_a_file_is_unreadable(tmp_path: Path) -> None:
    with pytest.raises(ControlInputError, match="Trial-balance file could not be read"):
        SourceSnapshot.capture(tmp_path, label="Trial-balance file")


@pytest.mark.parametrize(("header", "message"), [
    (",".join(CANONICAL_COLUMNS[:-1]), r"canonical schema mismatch \(missing YTDCredit\)"),
    (",".join((*CANONICAL_COLUMNS, "Memo")), r"canonical schema mismatch \(unexpected Memo\)"),
])
def test_schema_mismatch_names_only_what_differs(tmp_path: Path, header: str, message: str) -> None:
    path = tmp_path / "trial-balance.csv"
    path.write_text(header + "\n", encoding="utf-8")
    with pytest.raises(SchemaError, match=message):
        load_canonical_tb(path)


def test_basic_format_report_date_is_refused(tmp_path: Path) -> None:
    path = tmp_path / "trial-balance.csv"
    path.write_text(CURRENT.read_text(encoding="utf-8").replace("2026-07-31,", "20260731,"), encoding="utf-8")
    with pytest.raises(SchemaError, match="row 2 ReportDate must be an ISO date"):
        load_canonical_tb(path)


def test_header_only_trial_balance_is_refused(tmp_path: Path) -> None:
    path = tmp_path / "trial-balance.csv"
    path.write_text(",".join(CANONICAL_COLUMNS) + "\n", encoding="utf-8")
    with pytest.raises(SchemaError, match="no trial-balance rows were supplied"):
        load_canonical_tb(path)


def test_duplicate_mapping_account_is_refused(tmp_path: Path) -> None:
    path = tmp_path / "mapping.csv"
    path.write_text("AccountID,ReviewGroup\n100,Cash\n100,Debtors\n", encoding="utf-8")
    with pytest.raises(DuplicateKeyError, match="duplicate AccountID '100'"):
        load_mapping(path)


def test_duplicate_subledger_key_is_refused(tmp_path: Path) -> None:
    path = tmp_path / "subledger.csv"
    path.write_text("Tenant,AccountID,SubledgerBalance\nDemo,110,1.00\nDemo,110,2.00\n", encoding="utf-8")
    with pytest.raises(DuplicateKeyError, match="duplicate subledger key Tenant='Demo', AccountID='110'"):
        load_subledger(path)


def test_review_note_that_is_not_json_is_refused(tmp_path: Path) -> None:
    note = tmp_path / "note.json"
    note.write_text("reviewer: RD", encoding="utf-8")
    with pytest.raises(SchemaError, match="review note is not valid JSON"):
        load_reviewer_acknowledgement(note)


@pytest.mark.parametrize(("field", "value", "message"), [
    ("reviewer_initials", " ", "reviewer_initials must be a non-empty string of at most 12 characters"),
    ("reviewer_initials", "R" * 13, "reviewer_initials must be a non-empty string of at most 12 characters"),
    ("reviewer_initials", 7, "reviewer_initials must be a non-empty string of at most 12 characters"),
    ("comment", " ", "comment must be a non-empty string"),
    ("comment", None, "comment must be a non-empty string"),
    ("reviewed_on", 20260730, "reviewed_on must be an ISO date string"),
])
def test_review_note_field_gates(tmp_path: Path, field: str, value: object, message: str) -> None:
    values: dict[str, object] = {"reviewer_initials": "RD", "reviewed_on": "2026-07-30", "comment": "Reviewed."}
    values[field] = value
    note = tmp_path / "note.json"
    note.write_text(json.dumps(values), encoding="utf-8")
    with pytest.raises(SchemaError, match=message):
        load_reviewer_acknowledgement(note)


@pytest.mark.parametrize("payload", [
    '["RD", "2026-07-30", "Reviewed."]',
    '{"reviewer_initials": "RD", "reviewed_on": "2026-07-30"}',
])
def test_review_note_must_hold_exactly_the_three_members(tmp_path: Path, payload: str) -> None:
    note = tmp_path / "note.json"
    note.write_text(payload, encoding="utf-8")
    with pytest.raises(SchemaError, match="review note must contain exactly reviewer_initials, reviewed_on, and comment"):
        load_reviewer_acknowledgement(note)
