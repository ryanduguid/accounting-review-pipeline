from __future__ import annotations

import json
from decimal import Decimal
from pathlib import Path

import pytest
from closecontrol.errors import (
    ControlInputError,
    DateMismatchError,
    DuplicateKeyError,
    NumericGateError,
    SchemaError,
)
from closecontrol.loader import (
    load_canonical_tb,
    load_mapping,
    load_reviewer_acknowledgement,
    load_subledger,
    parse_money,
)

ROOT = Path(__file__).resolve().parents[1]


def test_typed_input_errors_subclass_the_flat_control_input_error() -> None:
    # Existing `except ControlInputError` handlers must keep catching every
    # typed gate, so each subclass stays inside the original hierarchy.
    for typed in (SchemaError, DuplicateKeyError, DateMismatchError, NumericGateError):
        assert issubclass(typed, ControlInputError)
        assert issubclass(typed, ValueError)


def test_duplicate_control_key_raises_the_duplicate_key_type(tmp_path: Path) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    duplicate = tmp_path / "duplicate.csv"
    lines = source.splitlines()
    duplicate.write_text("\n".join(lines + [lines[1]]) + "\n", encoding="utf-8")

    with pytest.raises(DuplicateKeyError, match="duplicate control key"):
        load_canonical_tb(duplicate)


def test_malformed_money_raises_the_numeric_gate_type() -> None:
    with pytest.raises(NumericGateError, match="invalid Debit"):
        parse_money("=1+1", field="Debit", row_number=2, path=Path("input.csv"))


def test_subledger_balances_pass_through_the_same_money_gate_as_tb_columns(
    tmp_path: Path,
) -> None:
    # load_subledger routes SubledgerBalance through parse_money, so the
    # injection and format gates that protect the TB money columns protect the
    # subledger too. This test pins that path: a refactor that reads the value
    # with Decimal() or float() directly would let a formula-leading or
    # malformed value into the reconciliation control.
    malformed = tmp_path / "malformed.csv"
    # A quoted "612,00": ambiguous European-style formatting that Decimal()
    # would reject inconsistently and float() would misread.
    malformed.write_text(
        'Tenant,AccountID,SubledgerBalance\nDemo Company,bank-guid,"612,00"\n',
        encoding="utf-8",
    )
    with pytest.raises(NumericGateError, match="invalid SubledgerBalance"):
        load_subledger(malformed)

    formula = tmp_path / "formula.csv"
    formula.write_text(
        "Tenant,AccountID,SubledgerBalance\nDemo Company,bank-guid,=1+1\n",
        encoding="utf-8",
    )
    with pytest.raises(NumericGateError, match="invalid SubledgerBalance"):
        load_subledger(formula)


def test_load_canonical_trial_balance_uses_stable_tenant_account_key() -> None:
    rows = load_canonical_tb(ROOT / "examples" / "current_trial_balance.csv")

    assert len(rows) == 7
    assert rows[0].key == ("Varrock Ventures Pty Ltd", "100")
    assert rows[0].ytd_net == parse_money("120000.00", field="test", row_number=1, path=Path("test"))


def test_loader_rejects_duplicate_control_key(tmp_path: Path) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    duplicate = tmp_path / "duplicate.csv"
    lines = source.splitlines()
    duplicate.write_text("\n".join(lines + [lines[1]]) + "\n", encoding="utf-8")

    with pytest.raises(ControlInputError, match="duplicate control key"):
        load_canonical_tb(duplicate)


@pytest.mark.parametrize("value", ["", "612,00", "1,2", "1 2", "1,234,56", "=1+1", "NaN"])
def test_amount_parser_rejects_ambiguous_or_formula_values(value: str) -> None:
    with pytest.raises(ControlInputError):
        parse_money(value, field="Debit", row_number=2, path=Path("input.csv"))


@pytest.mark.parametrize(
    "value", ["0.1234567890123456789", "123456789012345678.01", "9007199254740993.00"]
)
def test_amount_parser_keeps_every_supplied_digit(value: str) -> None:
    # Each of these values loses digits through a binary float. Money must reach
    # the controls as the exact decimal the source file supplied.
    parsed = parse_money(value, field="Debit", row_number=2, path=Path("input.csv"))

    assert isinstance(parsed, Decimal)
    assert parsed == Decimal(value)
    assert str(parsed) == value


def test_loader_accepts_exported_codeless_bank_account(tmp_path: Path) -> None:
    exported = tmp_path / "trial-balance.csv"
    exported.write_text(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,"
        "Debit,Credit,YTDDebit,YTDCredit\n"
        "2026-07-31,Demo Company,Assets,bank-guid,Business Bank Account,,"
        "0.00,0.00,125.00,0.00\n",
        encoding="utf-8",
    )

    [row] = load_canonical_tb(exported)

    assert row.key == ("Demo Company", "bank-guid")
    assert row.account_code == ""


@pytest.mark.parametrize("field", ["Tenant", "Section", "AccountID", "AccountName"])
def test_codeless_account_support_does_not_relax_required_identity_fields(
    tmp_path: Path, field: str
) -> None:
    columns = [
        "ReportDate", "Tenant", "Section", "AccountID", "AccountName",
        "AccountCode", "Debit", "Credit", "YTDDebit", "YTDCredit",
    ]
    values = [
        "2026-07-31", "Demo Company", "Assets", "bank-guid",
        "Business Bank Account", "", "0.00", "0.00", "125.00", "0.00",
    ]
    values[columns.index(field)] = ""
    exported = tmp_path / "trial-balance.csv"
    exported.write_text(
        ",".join(columns) + "\n" + ",".join(values) + "\n",
        encoding="utf-8",
    )

    with pytest.raises(ControlInputError, match=rf"empty {field}"):
        load_canonical_tb(exported)


@pytest.mark.parametrize(
    ("loader", "header", "row"),
    [
        (load_mapping, "AccountID,ReviewGroup", "bank-guid,Cash,unexpected"),
        (
            load_subledger,
            "Tenant,AccountID,SubledgerBalance",
            "Demo Company,bank-guid,125.00,unexpected",
        ),
    ],
)
def test_optional_csv_loaders_reject_surplus_cells(
    tmp_path: Path, loader: object, header: str, row: str
) -> None:
    csv_path = tmp_path / "optional.csv"
    csv_path.write_text(f"{header}\n{row}\n", encoding="utf-8")

    with pytest.raises(ControlInputError, match="more fields than its header"):
        loader(csv_path)  # type: ignore[operator]


def test_empty_report_date_is_reported_as_empty_not_invalid_iso(tmp_path: Path) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    blank_date = tmp_path / "blank_date.csv"
    blank_date.write_text(source.replace("2026-07-31,Varrock Ventures Pty Ltd,Assets,100", ",Varrock Ventures Pty Ltd,Assets,100"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="empty ReportDate"):
        load_canonical_tb(blank_date)


@pytest.mark.parametrize(
    ("old", "new", "message"),
    [
        ("2026-07-31,Varrock Ventures Pty Ltd,Assets,110", "2026-07-30,Varrock Ventures Pty Ltd,Assets,110", "one ReportDate"),
        ("2026-07-31,Varrock Ventures Pty Ltd,Assets,110", "2026-07-31,Other Tenant,Assets,110", "one tenant"),
    ],
)
def test_loader_rejects_mixed_period_or_tenant_scope(
    tmp_path: Path, old: str, new: str, message: str
) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    bad = tmp_path / "mixed.csv"
    bad.write_text(source.replace(old, new), encoding="utf-8")

    with pytest.raises(ControlInputError, match=message):
        load_canonical_tb(bad)


def test_loader_rejects_invisible_formatting_in_identifiers(tmp_path: Path) -> None:
    source = (ROOT / "examples" / "current_trial_balance.csv").read_text(encoding="utf-8")
    bad = tmp_path / "bidi.csv"
    bad.write_text(source.replace("Operating Bank", "Operating\u202e Bank"), encoding="utf-8")

    with pytest.raises(ControlInputError, match="control or formatting"):
        load_canonical_tb(bad)


@pytest.mark.parametrize("field", ["reviewer_initials", "comment"])
def test_review_note_rejects_invisible_formatting(tmp_path: Path, field: str) -> None:
    values = {"reviewer_initials": "RD", "reviewed_on": "2026-07-30", "comment": "Reviewed."}
    values[field] = values[field] + "\u202e"
    note = tmp_path / "note.json"
    note.write_text(json.dumps(values), encoding="utf-8")

    with pytest.raises(ControlInputError, match="control or formatting"):
        load_reviewer_acknowledgement(note)


def test_review_note_accepts_plain_text(tmp_path: Path) -> None:
    note = tmp_path / "note.json"
    note.write_text(
        json.dumps({"reviewer_initials": "RD", "reviewed_on": "2026-07-30", "comment": "Reviewed."}),
        encoding="utf-8",
    )

    acknowledgement = load_reviewer_acknowledgement(note)

    assert acknowledgement is not None
    assert acknowledgement.reviewer_initials == "RD"


def test_review_note_with_utf8_bom_parses_same_as_without(tmp_path: Path) -> None:
    payload = json.dumps(
        {"reviewer_initials": "RD", "reviewed_on": "2026-07-30", "comment": "Reviewed."}
    )
    plain = tmp_path / "note.json"
    plain.write_text(payload, encoding="utf-8")
    with_bom = tmp_path / "note_bom.json"
    with_bom.write_text(payload, encoding="utf-8-sig")

    assert load_reviewer_acknowledgement(with_bom) == load_reviewer_acknowledgement(plain)



@pytest.mark.parametrize("field,value", [
    ("reviewer_initials", '"RD"'),
    ("reviewed_on", '"2026-04-12"'),
    ("comment", '"Reviewed fabricated facts."'),
])
@pytest.mark.parametrize("escaped", [False, True])
def test_audit_review_note_rejects_duplicate_members(tmp_path, field, value, escaped):
    payload = '{"reviewer_initials":"RD","reviewed_on":"2026-04-12","comment":"Reviewed fabricated facts."}'
    duplicate = field if not escaped else "\\u" + f"{ord(field[0]):04x}" + field[1:]
    payload = payload.replace(f'"{field}":{value}', f'"{field}":{value},"{duplicate}":{value}')
    note = tmp_path / "duplicate.json"
    note.write_text(payload, encoding="utf-8")
    with pytest.raises(SchemaError, match="more than once"):
        load_reviewer_acknowledgement(note)


def test_audit_review_note_rejects_nested_duplicate_members(tmp_path):
    note = tmp_path / "nested.json"
    note.write_text('{"reviewer_initials":"RD","reviewed_on":"2026-04-12",'
                    '"comment":{"nested":[{"x":1,"x":2}]}}', encoding="utf-8")
    with pytest.raises(SchemaError, match="more than once"):
        load_reviewer_acknowledgement(note)


@pytest.mark.parametrize("payload,diagnostic", [
    ('{"comment":' + '1' * 5000 + '}', "could not be read"),
    ('[' * 2000 + '0' + ']' * 2000, "nested too deeply|must contain exactly"),
], ids=["huge-integer", "deep-arrays"])
def test_audit_review_note_normalises_interpreter_limits(tmp_path, payload, diagnostic):
    note = tmp_path / "limits.json"
    note.write_text(payload, encoding="utf-8")
    with pytest.raises(SchemaError, match=diagnostic):
        load_reviewer_acknowledgement(note)



def test_audit_review_note_translates_decoder_recursion(tmp_path, monkeypatch):
    note = tmp_path / "note.json"
    note.write_text("{}", encoding="utf-8")

    def too_deep(*args, **kwargs):
        raise RecursionError("fabricated decoder limit")

    monkeypatch.setattr(json, "loads", too_deep)
    with pytest.raises(SchemaError, match="nested too deeply"):
        load_reviewer_acknowledgement(note)
