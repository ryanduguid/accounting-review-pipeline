"""Coding evidence must discriminate purposes and retain missing proof and original postings."""
import csv
import hashlib
import json
from decimal import localcontext

import pytest
from closecontrol.classification import classify
from closecontrol.cli import main
from closecontrol.errors import ControlInputError
from closecontrol.reconciliation import COLUMNS


def _rows():
    return [{"Tenant": "Synthetic", "AccountID": "610", "Currency": "AUD",
             "TransactionID": f"{kind}-{month}", "Date": f"2025-{month:02d}-15",
             "Reference": f"bill-{kind}-{month}", "Description": "Recurring supplier fee",
             "Debit": "150.00", "Credit": "0"}
            for month in range(1, 13) for kind in ("software", "office")]


def _write(path, rows):
    with path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, COLUMNS, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def _fixture(tmp_path, rows=None, current=None):
    rows = _rows() if rows is None else rows
    before, after, path = (tmp_path / name for name in ("original.csv", "current.csv", "evidence.json"))
    _write(before, rows)
    _write(after, rows if current is None else current)
    document = {"schema_version": 1, "tenant": "Synthetic", "currency": "AUD",
                "period_start": "2025-01-01", "period_end": "2025-12-31",
                "original_sha256": hashlib.sha256(before.read_bytes()).hexdigest(),
                "current_sha256": hashlib.sha256(after.read_bytes()).hexdigest(),
                "expectations": [{"original_account_id": row["AccountID"],
                                  "transaction_id": row["TransactionID"],
                                  "current_account_id": row["AccountID"],
                                  "expected_account_id": "620" if row["TransactionID"].startswith("software") else "610",
                                  "purpose": "Software access" if row["TransactionID"].startswith("software") else "Office stationery",
                                  "evidence_reference": row["Reference"] + " synthetic item"} for row in rows]}
    path.write_text(json.dumps(document), encoding="utf-8")
    return before, after, path, document


def _run(inputs):
    return classify(*inputs[:3], currency="AUD")


def test_twelve_mispostings_are_distinguished_from_identical_recurring_expenses(tmp_path):
    inputs = _fixture(tmp_path)
    original = inputs[0].read_bytes()
    result = _run(inputs)
    assert result["status"] == "REVIEW"
    assert result["original_count"] == result["current_count"] == 24
    wrong = [row for row in result["items"] if row["status"] == "REVIEW"]
    assert len(wrong) == 12
    assert all(row["reasons"] == ["CODING_DIFFERENCE"] for row in wrong)
    assert all(row["original_disagrees_with_evidence"] for row in wrong)
    assert inputs[0].read_bytes() == original
    assert result["source_sha256"]["original"] == inputs[3]["original_sha256"]


def test_explicit_recode_keeps_original_discrepancy(tmp_path):
    rows = _rows()[:2]
    changed = [dict(row) for row in rows]
    changed[0]["AccountID"] = "620"
    inputs = _fixture(tmp_path, rows, changed)
    inputs[3]["expectations"][0]["current_account_id"] = "620"
    inputs[2].write_text(json.dumps(inputs[3]))
    result = _run(inputs)
    assert result["status"] == "PASS"
    first = next(row for row in result["items"] if row["original"]["TransactionID"].startswith("software"))
    assert first["original"]["AccountID"] == "610"
    assert first["current"]["AccountID"] == "620"
    assert first["original_disagrees_with_evidence"] is True


@pytest.mark.parametrize("field", ["expected_account_id", "purpose", "evidence_reference", "whole_assertion"])
def test_missing_evidence_never_inherits_the_observed_code(tmp_path, field):
    inputs = _fixture(tmp_path, _rows()[1:2])
    if field == "whole_assertion":
        inputs[3]["expectations"] = []
    else:
        inputs[3]["expectations"][0][field] = ""
    inputs[2].write_text(json.dumps(inputs[3]))
    result = _run(inputs)
    assert result["status"] == "REVIEW"
    assert result["items"][0]["original_disagrees_with_evidence"] is None
    assert "MISSING_CLASSIFICATION_EVIDENCE" in result["items"][0]["reasons"]


@pytest.mark.parametrize("change", ["unbound_recode", "date", "reference", "description", "amount", "removed", "added"])
def test_changed_transaction_facts_and_population_remain_unresolved(tmp_path, change):
    rows = _rows()[1:2]
    changed = [dict(row) for row in rows]
    if change == "unbound_recode":
        changed[0]["AccountID"] = "620"
    elif change == "date":
        changed[0]["Date"] = "2025-01-16"
    elif change == "reference":
        changed[0]["Reference"] = "different bill"
    elif change == "description":
        changed[0]["Description"] = "Annual software licence"
    elif change == "amount":
        changed[0]["Debit"] = "151"
    elif change == "removed":
        changed = []
    else:
        changed.append(dict(changed[0], TransactionID="unmatched"))
    result = _run(_fixture(tmp_path, rows, changed))
    assert result["status"] == "REVIEW"
    assert result["items"][0]["reasons"] or result["unmatched_current"]
    if change == "description":
        assert result["items"][0]["reasons"] == ["TRANSACTION_FACTS_CHANGED"]
        assert result["items"][0]["original"]["Description"] == "Recurring supplier fee"
        assert result["items"][0]["current"]["Description"] == "Annual software licence"


def test_transaction_id_alone_cannot_select_another_accounts_posting(tmp_path):
    row = _rows()[1]
    other = dict(row, AccountID="620")
    inputs = _fixture(tmp_path, [row, other])
    inputs[3]["expectations"][1]["current_account_id"] = "610"
    inputs[2].write_text(json.dumps(inputs[3]))
    result = _run(inputs)
    assert result["status"] == "REVIEW"
    assert all(item["status"] == "REVIEW" for item in result["items"])
    assert result["items"][1]["reasons"] == ["CURRENT_TRANSACTION_REUSED"]
    assert result["unmatched_current"][0]["AccountID"] == "620"


@pytest.mark.parametrize("accounts", [("010", "999"), ("999", "010")])
def test_all_reused_mapping_participants_remain_unresolved(tmp_path, accounts):
    row = _rows()[1]
    inputs = _fixture(tmp_path, [dict(row, AccountID=account) for account in accounts], [row])
    for policy in inputs[3]["expectations"]:
        policy["current_account_id"] = "610"
    inputs[2].write_text(json.dumps(inputs[3]))
    result = _run(inputs)
    assert result["status"] == "REVIEW"
    assert all(item["reasons"] == ["CURRENT_TRANSACTION_REUSED"] for item in result["items"])
    assert all(item["original_disagrees_with_evidence"] for item in result["items"])
    assert result["original_count"] == 2 and result["current_count"] == 1
    assert not result["unmatched_current"]


@pytest.mark.parametrize("change", ["quote", "duplicate", "short"])
def test_malformed_csv_cannot_be_repaired_into_passing_evidence(tmp_path, change):
    inputs = _fixture(tmp_path, _rows()[1:2])
    path = inputs[1]
    if change == "quote":
        path.write_bytes(path.read_bytes().rstrip(b"\n").rsplit(b",", 1)[0] + b',"0')
    elif change == "duplicate":
        path.write_bytes(path.read_bytes().replace(b"Debit,Credit", b"Debit,Credit,Credit").replace(b"150.00,0", b"150.00,NaN,0"))
    else:
        columns = [name for name in COLUMNS if name != "Description"] + ["Description"]
        values = [inputs[3]["tenant"], "610", "AUD", "office-1", "2025-01-15", "bill-office-1", "150.00", "0"]
        path.write_text(",".join(columns) + "\n" + ",".join(values) + "\n")
    inputs[3]["current_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    inputs[2].write_text(json.dumps(inputs[3]))
    with pytest.raises(ControlInputError):
        _run(inputs)
    assert main(["classify", "--original-transactions", str(inputs[0]), "--current-transactions", str(path),
                 "--coding-evidence", str(inputs[2]), "--currency", "AUD"]) == 1


@pytest.mark.parametrize("newline", ["\n", "\r\n"])
def test_valid_quoted_csv_and_explicit_empty_optional_cells(tmp_path, newline):
    inputs = _fixture(tmp_path, [dict(_rows()[1], Reference="", Description="Office, stationery", Debit="$1,234.56")])
    for path in inputs[:2]:
        with path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, COLUMNS, lineterminator=newline)
            writer.writeheader()
            writer.writerow(dict(_rows()[1], Reference="", Description="Office, stationery", Debit="$1,234.56"))
    inputs[3]["original_sha256"] = hashlib.sha256(inputs[0].read_bytes()).hexdigest()
    inputs[3]["current_sha256"] = hashlib.sha256(inputs[1].read_bytes()).hexdigest()
    inputs[2].write_text(json.dumps(inputs[3]))
    assert _run(inputs)["status"] == "PASS"


@pytest.mark.parametrize("change", ["digest", "duplicate_assertion", "unknown", "extra", "boolean_version",
                                   "currency", "tenant", "period", "bad_date", "nontext"])
def test_malformed_or_unbound_assertions_are_refused(tmp_path, change):
    inputs = _fixture(tmp_path, _rows()[1:2])
    doc = inputs[3]
    if change == "digest":
        doc["original_sha256"] = "0" * 64
    elif change == "duplicate_assertion":
        doc["expectations"].append(doc["expectations"][0])
    elif change == "unknown":
        doc["expectations"][0]["transaction_id"] = "absent"
    elif change == "extra":
        doc["expectations"][0]["approved"] = True
    elif change == "boolean_version":
        doc["schema_version"] = True
    elif change == "currency":
        doc["currency"] = "USD"
    elif change == "tenant":
        doc["tenant"] = "Other"
    elif change == "period":
        doc["period_end"] = "2024-12-31"
    elif change == "bad_date":
        doc["period_start"] = "2025-02-30"
    else:
        doc["expectations"][0]["purpose"] = 1
    inputs[2].write_text(json.dumps(doc))
    with pytest.raises(ControlInputError):
        _run(inputs)


def test_duplicate_json_keys_and_changed_bytes_are_refused(tmp_path):
    inputs = _fixture(tmp_path)
    inputs[2].write_text(inputs[2].read_text().replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1'))
    with pytest.raises(ControlInputError):
        _run(inputs)
    inputs[2].write_text(json.dumps(inputs[3]))
    inputs[1].write_bytes(inputs[1].read_bytes() + b"\n")
    with pytest.raises(ControlInputError, match="digests"):
        _run(inputs)


def test_empty_selection_and_missing_mapping_do_not_pass(tmp_path):
    assert _run(_fixture(tmp_path, []))["status"] == "REVIEW"


def test_exact_money_and_accounting_format_survive_low_ambient_precision(tmp_path):
    row = dict(_rows()[1], Debit="$123,456,789.12")
    inputs = _fixture(tmp_path, [row])
    with localcontext() as context:
        context.prec = 3
        result = _run(inputs)
    assert result["status"] == "PASS"
    assert result["items"][0]["original"]["Debit"] == "123456789.12"


def test_cli_exit_contract(tmp_path, capsys):
    inputs = _fixture(tmp_path)
    args = ["classify", "--original-transactions", str(inputs[0]),
            "--current-transactions", str(inputs[1]), "--coding-evidence", str(inputs[2]),
            "--currency", "AUD"]
    assert main(args) == 2
    assert json.loads(capsys.readouterr().out)["status"] == "REVIEW"
    _fixture(tmp_path, _rows()[1:2])
    assert main(args) == 0
    assert json.loads(capsys.readouterr().out)["status"] == "PASS"
    inputs[2].write_text("invalid")
    assert main(args) == 1
    assert "Invalid coding evidence" in capsys.readouterr().err
