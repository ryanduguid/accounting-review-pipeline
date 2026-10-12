"""Equity schedule gates and the pack-side checks on stored equity evidence."""
from __future__ import annotations

import copy

import pytest
from closecontrol.engine import review_close
from closecontrol.equity import load_schedule, validate_result
from closecontrol.errors import ControlInputError
from closecontrol.loader import SourceSnapshot

from test_equity import _change, _inputs

TOO_PRECISE = "1234567890123456789012345678.9"


def _load(inputs):
    return load_schedule(SourceSnapshot.capture(inputs["equity_schedule_path"], label="Equity schedule"))


@pytest.mark.parametrize("field,value,message", [
    ("opening_date", "20260831", "Equity opening_date must use YYYY-MM-DD"),
    ("currency", "aud", "Equity currency must be an uppercase 3-letter code"),
    ("currency", 36, "Equity currency must be an uppercase 3-letter code"),
    ("closing_date", "2026-08-31", "Equity closing_date must follow opening_date"),
    ("prior_source_sha256", "A" * 64, "Equity prior_source_sha256 must be a lowercase SHA-256 digest"),
    ("current_source_sha256", None, "Equity current_source_sha256 must be a lowercase SHA-256 digest"),
    ("equity_account_ids", [], "Equity equity_account_ids must be a non-empty list"),
    ("equity_account_ids", "E", "Equity equity_account_ids must be a non-empty list"),
    ("movements", {}, "Equity movements must be a list"),
])
def test_schedule_field_gates(tmp_path, field, value, message):
    inputs, document = _inputs(tmp_path)
    document[field] = value
    _change(inputs, document)
    with pytest.raises(ControlInputError, match=message):
        _load(inputs)


def test_currency_evidence_with_control_characters_is_refused(tmp_path):
    inputs, _ = _inputs(tmp_path)
    inputs["equity_currency_evidence"] = "Both reports\x07"
    with pytest.raises(ControlInputError, match="Currency evidence must not contain control characters"):
        review_close(**inputs)


def test_equity_arithmetic_beyond_exact_precision_is_refused(tmp_path):
    movement = {"movement_id": "big", "date": "2026-09-30", "account_id": "E",
                "debit": "0", "credit": TOO_PRECISE, "description": "Oversized",
                "evidence_reference": "Synthetic record"}
    inputs, _ = _inputs(tmp_path, opening={"E": "1"}, closing={"E": "1"}, movements=[movement])
    with pytest.raises(ControlInputError, match="Equity arithmetic exceeds supported exact decimal precision"):
        review_close(**inputs)


def _evidence(tmp_path):
    """Return a REVIEW equity result with the sources and exceptions a pack would hold."""
    inputs, _ = _inputs(tmp_path)
    pack = review_close(**inputs)
    result = copy.deepcopy(pack.equity_reconciliation)
    sources = {"equity_schedule": result["schedule_sha256"]}
    exceptions = [{"control": row.control, "status": row.status} for row in pack.exceptions]
    return result, sources, exceptions


def test_stored_review_evidence_is_accepted(tmp_path):
    result, sources, exceptions = _evidence(tmp_path)
    assert validate_result(result, sources, exceptions) is result  # nosec B101: pytest outcome assertion


def _mutate(result, sources, exceptions, mutation):
    account, movement = result["accounts"][0], result["movements"][0]
    if mutation == "not_object":
        return [result], sources, exceptions
    if mutation == "field_type":
        result["tenant"] = 1
    elif mutation == "provenance":
        sources = {}
    elif mutation == "account_shape":
        result["accounts"][0] = "E"
    elif mutation == "account_identity":
        account["account_id"] = 1
    elif mutation == "account_amount_type":
        account["opening"] = 100000
    elif mutation == "total_shape":
        result["total"] = {"opening": "100000"}
    elif mutation == "movement_shape":
        result["movements"][0] = "capital"
    elif mutation == "movement_field_type":
        movement["debit"] = 0
    elif mutation == "issues_on_calculated":
        result["issues"] = ["Unexplained issue"]
    elif mutation == "duplicate_movement":
        result["movements"].append(dict(movement))
    elif mutation == "two_sided_movement":
        movement["debit"] = "1"
    elif mutation == "orphan_movement":
        result["movements"].append({**movement, "movement_id": "orphan", "account_id": "X"})
    elif mutation == "too_precise":
        account["opening"] = TOO_PRECISE
    else:
        exceptions = []
    return result, sources, exceptions


@pytest.mark.parametrize("mutation,message", [
    ("not_object", "Malformed equity reconciliation result"),
    ("field_type", "Equity result fields must be strings"),
    ("provenance", "Equity schedule provenance disagrees with the pack"),
    ("account_shape", "Malformed equity account evidence"),
    ("account_identity", "Malformed equity account identity"),
    ("account_amount_type", "Equity account amounts must be decimal strings"),
    ("total_shape", "Malformed equity total evidence"),
    ("movement_shape", "Malformed equity movement evidence"),
    ("movement_field_type", "Equity movement fields must be strings"),
    ("issues_on_calculated", "Calculated equity evidence requires complete accounts and totals"),
    ("duplicate_movement", "Duplicate equity movement evidence"),
    ("two_sided_movement", "Equity movement must have exactly one positive side"),
    ("orphan_movement", "Equity movement references an unknown account"),
    ("too_precise", "Equity evidence exceeds supported exact decimal precision"),
    ("status_without_findings", "Equity result status disagrees with its findings"),
])
def test_stored_evidence_gates(tmp_path, mutation, message):
    result, sources, exceptions = _mutate(*_evidence(tmp_path), mutation)
    with pytest.raises(ControlInputError, match=message):
        validate_result(result, sources, exceptions)


def test_blocked_evidence_must_not_carry_balances(tmp_path):
    result, sources, exceptions = _evidence(tmp_path)
    result["status"] = "BLOCKED"
    result["issues"] = ["Schedule tenant does not match the supplied trial-balance snapshots."]
    with pytest.raises(ControlInputError, match="Blocked equity evidence must omit calculated balances"):
        validate_result(result, sources, exceptions)
