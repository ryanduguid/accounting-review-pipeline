from __future__ import annotations

import json
from decimal import Decimal
from pathlib import Path

import pytest

from tests.support import EXAMPLES, copy_example_pack
from reviewready.engine import review_pack
from reviewready.errors import (
    DateMismatchError,
    DuplicateKeyError,
    GateInputError,
    NumericGateError,
    SchemaError,
)
from reviewready.loader import (
    SourceSnapshot,
    load_canonical_tb,
    load_open_items,
    load_reviewer_acknowledgement,
    load_self_review,
    parse_iso_date,
    parse_money,
)


def _snapshot(path: Path) -> SourceSnapshot:
    return SourceSnapshot.capture(path, label="test file")


def test_typed_input_errors_subclass_the_flat_gate_input_error() -> None:
    for typed in (SchemaError, DuplicateKeyError, DateMismatchError, NumericGateError):
        assert issubclass(typed, GateInputError)
        assert issubclass(typed, ValueError)


def test_duplicate_control_key_raises_the_duplicate_key_type(tmp_path: Path) -> None:
    source = (EXAMPLES / "bas-ready" / "trial_balance.csv").read_text(encoding="utf-8")
    duplicate = tmp_path / "duplicate.csv"
    lines = source.splitlines()
    duplicate.write_text("\n".join(lines + [lines[1]]) + "\n", encoding="utf-8")

    with pytest.raises(DuplicateKeyError, match="duplicate control key"):
        load_canonical_tb(_snapshot(duplicate))


@pytest.mark.parametrize("value", ["", "612,00", "1,2", "1 2", "=1+1", "NaN"])
def test_amount_parser_rejects_ambiguous_or_formula_values(value: str) -> None:
    with pytest.raises(NumericGateError):
        parse_money(value, field="Debit", row_number=2, path=Path("input.csv"))


@pytest.mark.parametrize(
    "value", ["0.1234567890123456789", "123456789012345678.01", "9007199254740993.00"]
)
def test_amount_parser_keeps_every_supplied_digit(value: str) -> None:
    parsed = parse_money(value, field="Debit", row_number=2, path=Path("input.csv"))

    assert isinstance(parsed, Decimal)
    assert parsed == Decimal(value)
    assert str(parsed) == value


@pytest.mark.parametrize("value", ["20260405", "2026-W14-2"])
def test_date_parser_rejects_forms_only_some_interpreters_accept(value: str) -> None:
    with pytest.raises(SchemaError, match="must be an ISO date"):
        parse_iso_date(value, field="ReportDate", path=Path("input.csv"), row_number=2)


def test_empty_report_date_is_reported_as_empty_not_invalid_iso(tmp_path: Path) -> None:
    source = (EXAMPLES / "bas-ready" / "trial_balance.csv").read_text(encoding="utf-8")
    blank = tmp_path / "blank_date.csv"
    blank.write_text(
        source.replace("2026-03-31,Cedar and Pine Consulting Pty Ltd", ",Cedar and Pine Consulting Pty Ltd"),
        encoding="utf-8",
    )

    with pytest.raises(SchemaError, match="empty ReportDate"):
        load_canonical_tb(_snapshot(blank))


def test_loader_rejects_mixed_tenant_scope(tmp_path: Path) -> None:
    source = (EXAMPLES / "bas-ready" / "trial_balance.csv").read_text(encoding="utf-8")
    mixed = tmp_path / "mixed.csv"
    mixed.write_text(
        source.replace(
            "2026-03-31,Cedar and Pine Consulting Pty Ltd,Liabilities",
            "2026-03-31,Other Tenant,Liabilities",
            1,
        ),
        encoding="utf-8",
    )

    with pytest.raises(SchemaError, match="one tenant"):
        load_canonical_tb(_snapshot(mixed))


def test_loader_rejects_invisible_formatting_in_identifiers(tmp_path: Path) -> None:
    source = (EXAMPLES / "bas-ready" / "trial_balance.csv").read_text(encoding="utf-8")
    bad = tmp_path / "bidi.csv"
    bad.write_text(
        source.replace("Operating Bank", "Operating" + chr(0x202E) + " Bank"),
        encoding="utf-8",
    )

    with pytest.raises(SchemaError, match="control or formatting"):
        load_canonical_tb(_snapshot(bad))


def test_open_items_reject_surplus_cells(tmp_path: Path) -> None:
    csv_path = tmp_path / "open_items.csv"
    csv_path.write_text(
        "ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n"
        "OI-001,EXPLAIN,AB,2026-04-05,CLEARED,Late worksheet,Filed,extra\n",
        encoding="utf-8",
    )

    with pytest.raises(SchemaError, match="more fields than its header"):
        load_open_items(_snapshot(csv_path))


def test_self_review_must_match_selected_profile(tmp_path: Path) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    payload = json.loads((dest / "self_review.json").read_text(encoding="utf-8"))
    payload["engagement_type"] = "month_end"
    (dest / "self_review.json").write_text(json.dumps(payload), encoding="utf-8")

    with pytest.raises(SchemaError, match="must match the selected profile 'bas'"):
        load_self_review(_snapshot(dest / "self_review.json"), expected_profile="bas")


@pytest.mark.parametrize("field", ["reviewer_initials", "comment"])
def test_review_note_rejects_invisible_formatting(tmp_path: Path, field: str) -> None:
    values = {
        "reviewer_initials": "RD",
        "reviewed_on": "2026-04-12",
        "comment": "Reviewed.",
    }
    values[field] = values[field] + chr(0x202E)
    note = tmp_path / "note.json"
    note.write_text(json.dumps(values), encoding="utf-8")

    with pytest.raises(SchemaError, match="control or formatting"):
        load_reviewer_acknowledgement(note)


def test_review_note_with_utf8_bom_parses_same_as_without(tmp_path: Path) -> None:
    payload = json.dumps(
        {
            "reviewer_initials": "RD",
            "reviewed_on": "2026-04-12",
            "comment": "Reviewed fabricated demo findings only.",
        }
    )
    plain = tmp_path / "note.json"
    plain.write_text(payload, encoding="utf-8")
    with_bom = tmp_path / "note_bom.json"
    with_bom.write_text(payload, encoding="utf-8-sig")

    assert load_reviewer_acknowledgement(with_bom) == load_reviewer_acknowledgement(plain)


def test_review_pack_uses_the_fabricated_bas_tenant() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready")
    current = (EXAMPLES / "bas-ready" / "trial_balance.csv").read_text(encoding="utf-8")
    assert "Cedar and Pine Consulting Pty Ltd" in current
    assert pack.period_end == "2026-03-31"
