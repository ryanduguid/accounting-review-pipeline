"""Reconcile independent movement records against selected ledger equity balances."""
from __future__ import annotations

import hashlib
import html
import json
import re
from datetime import date
from decimal import Context, Decimal, Inexact, InvalidOperation, localcontext
from typing import Any

from .errors import ControlInputError
from .loader import SourceSnapshot
from .models import ExceptionItem, Status, TrialBalanceRow

_FIELDS = {"schema_version", "tenant", "opening_date", "closing_date", "currency", "basis",
           "prior_source_sha256", "current_source_sha256", "equity_account_ids", "complete",
           "movements"}
_MOVEMENT_FIELDS = {"movement_id", "date", "account_id", "debit", "credit",
                    "description", "evidence_reference"}
_RESULT_FIELDS = {"schema_version", "basis", "tenant", "opening_date", "closing_date",
                  "currency", "currency_evidence", "schedule_sha256", "status", "issues",
                  "accounts", "total", "movements", "tolerance", "complete"}
_NUMBERS = ("opening", "movement", "expected_close", "actual_close", "unexplained")


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ControlInputError(f"Duplicate equity JSON field: {key}.")
        result[key] = value
    return result


def _text(value: Any, label: str) -> str:
    if (not isinstance(value, str) or not value.strip()
            or any(ord(char) < 32 for char in value)):
        raise ControlInputError(f"Equity {label} must be non-empty text without control characters.")
    return value


def _date(value: Any, label: str) -> date:
    text = _text(value, label)
    try:
        result = date.fromisoformat(text)
    except ValueError as exc:
        raise ControlInputError(f"Equity {label} must be an ISO date.") from exc
    if result.isoformat() != text:
        raise ControlInputError(f"Equity {label} must use YYYY-MM-DD.")
    return result


def _amount(value: Any, label: str) -> Decimal:
    if (not isinstance(value, str) or len(value) > 64
            or re.fullmatch(r"\d+(?:\.\d+)?", value, flags=re.ASCII) is None):
        raise ControlInputError(f"Equity {label} must be a non-negative decimal string.")
    return Decimal(value)


def load_schedule(snapshot: SourceSnapshot) -> dict[str, Any]:
    try:
        document = json.loads(snapshot.text(label="Equity schedule", encoding="utf-8-sig"),
                              object_pairs_hook=_object)
    except (ValueError, UnicodeError) as exc:
        raise ControlInputError(f"Invalid equity schedule: {exc}") from exc
    if not isinstance(document, dict) or set(document) != _FIELDS:
        raise ControlInputError("Equity schedule must contain exactly the documented fields.")
    if type(document["schema_version"]) is not int or document["schema_version"] != 1:
        raise ControlInputError("Equity schema_version must be integer 1.")
    if document["basis"] != "ledger_equity" or type(document["complete"]) is not bool:
        raise ControlInputError("Equity basis must be ledger_equity and complete must be boolean.")
    _text(document["tenant"], "tenant")
    if not isinstance(document["currency"], str) or not re.fullmatch("[A-Z]{3}", document["currency"]):
        raise ControlInputError("Equity currency must be an uppercase 3-letter code.")
    opening = _date(document["opening_date"], "opening_date")
    closing = _date(document["closing_date"], "closing_date")
    if closing <= opening:
        raise ControlInputError("Equity closing_date must follow opening_date.")
    for key in ("prior_source_sha256", "current_source_sha256"):
        if not isinstance(document[key], str) or not re.fullmatch("[0-9a-f]{64}", document[key]):
            raise ControlInputError(f"Equity {key} must be a lowercase SHA-256 digest.")
    accounts = document["equity_account_ids"]
    if not isinstance(accounts, list) or not accounts:
        raise ControlInputError("Equity equity_account_ids must be a non-empty list.")
    account_ids = [_text(value, "account ID") for value in accounts]
    if len(set(account_ids)) != len(account_ids):
        raise ControlInputError("Equity account IDs must be unique.")
    if not isinstance(document["movements"], list):
        raise ControlInputError("Equity movements must be a list.")
    seen: set[str] = set()
    for row in document["movements"]:
        if not isinstance(row, dict) or set(row) != _MOVEMENT_FIELDS:
            raise ControlInputError("Equity movement must contain exactly the documented fields.")
        for key in ("movement_id", "account_id", "description", "evidence_reference"):
            _text(row[key], key)
        if row["movement_id"] in seen:
            raise ControlInputError("Equity movement IDs must be unique.")
        seen.add(row["movement_id"])
        if row["account_id"] not in account_ids:
            raise ControlInputError("Equity movement account is outside the selected accounts.")
        if not opening < _date(row["date"], "movement date") <= closing:
            raise ControlInputError("Equity movement date is outside the declared interval.")
        debit, credit = _amount(row["debit"], "debit"), _amount(row["credit"], "credit")
        if (debit > 0) == (credit > 0):
            raise ControlInputError("Equity movement must have exactly one positive side.")
    return document


def _finding(status: Status, reason: str, row: TrialBalanceRow | None = None,
             expected: Decimal | None = None, actual: Decimal | None = None,
             difference: Decimal | None = None, tolerance: Decimal | None = None) -> ExceptionItem:
    return ExceptionItem(
        control="equity_reconciliation", status=status, tenant=row.tenant if row else "",
        account_id=row.account_id if row else "", account_code=row.account_code if row else "",
        account_name=row.account_name if row else "", current_value=actual, prior_value=expected,
        difference=difference, threshold=tolerance, percentage_change=None, reason=reason,
        reviewer_action="Reconcile the independent movement records and retained source evidence.",
    )


def reconcile_equity(snapshot: SourceSnapshot, *, prior: list[TrialBalanceRow],
                     current: list[TrialBalanceRow], prior_sha256: str, current_sha256: str,
                     currency: str | None, currency_evidence: str | None,
                     tolerance: Decimal) -> tuple[dict[str, Any], list[ExceptionItem]]:
    document = load_schedule(snapshot)
    issues = []
    for key, expected in (
        ("tenant", current[0].tenant), ("opening_date", prior[0].report_date.isoformat()),
        ("closing_date", current[0].report_date.isoformat()),
        ("prior_source_sha256", prior_sha256), ("current_source_sha256", current_sha256),
    ):
        if document[key] != expected:
            issues.append(f"Schedule {key} does not match the supplied trial-balance snapshots.")
    if not document["complete"]:
        issues.append("The preparer has not declared the selected schedule complete.")
    if not currency or not re.fullmatch("[A-Z]{3}", currency) or currency != document["currency"]:
        issues.append("Independent currency confirmation is missing or differs from the schedule.")
    if not currency_evidence or not currency_evidence.strip():
        issues.append("Independent trial-balance currency evidence is missing.")
    elif any(ord(char) < 32 for char in currency_evidence):
        raise ControlInputError("Currency evidence must not contain control characters.")
    old = {row.account_id: row for row in prior}
    new = {row.account_id: row for row in current}
    for key in document["equity_account_ids"]:
        if key not in old or key not in new:
            issues.append(f"Account {key!r} must be present in both trial balances.")
        elif old[key].section != new[key].section or new[key].section.casefold() != "equity":
            issues.append(f"Account {key!r} must retain an explicit Equity section in both snapshots.")
    result: dict[str, Any] = {
        "schema_version": 1, "basis": "ledger_equity", "tenant": document["tenant"],
        "opening_date": document["opening_date"], "closing_date": document["closing_date"],
        "currency": currency or "", "currency_evidence": currency_evidence or "",
        "schedule_sha256": snapshot.sha256, "complete": document["complete"],
        "status": "BLOCKED" if issues else "PASS", "issues": issues, "accounts": [], "total": None,
        "movements": document["movements"], "tolerance": format(tolerance, "f"),
    }
    if issues:
        return result, [_finding("BLOCKED", " ".join(issues))]
    findings = []
    try:
        with localcontext(Context(prec=28)) as context:
            context.traps[Inexact] = True
            for key in sorted(document["equity_account_ids"]):
                opening = old[key].ytd_credit - old[key].ytd_debit
                rows = [row for row in document["movements"] if row["account_id"] == key]
                movement = sum((Decimal(row["credit"]) - Decimal(row["debit"])
                                for row in rows), Decimal(0))
                expected_close = opening + movement
                actual = new[key].ytd_credit - new[key].ytd_debit
                difference = actual - expected_close
                result["accounts"].append({
                    "account_id": key, "opening": format(opening, "f"),
                    "movement": format(movement, "f"), "expected_close": format(expected_close, "f"),
                    "actual_close": format(actual, "f"), "unexplained": format(difference, "f"),
                    "movement_ids": [row["movement_id"] for row in rows],
                })
                if abs(difference) > tolerance:
                    findings.append(_finding(
                        "REVIEW", f"Ledger equity differs from its independent movement schedule by "
                        f"{difference}; tolerance {tolerance}.", new[key], expected_close, actual,
                        difference, tolerance))
            total = {name: sum((Decimal(row[name]) for row in result["accounts"]), Decimal(0))
                     for name in _NUMBERS}
            result["total"] = {key: format(value, "f") for key, value in total.items()}
            if abs(total["unexplained"]) > tolerance and not findings:
                findings.append(_finding(
                    "REVIEW", "Aggregate equity difference exceeds the reconciliation tolerance.",
                    expected=total["expected_close"], actual=total["actual_close"],
                    difference=total["unexplained"], tolerance=tolerance))
    except (Inexact, InvalidOperation) as exc:
        raise ControlInputError("Equity arithmetic exceeds supported exact decimal precision.") from exc
    result["status"] = "REVIEW" if findings else "PASS"
    return result, findings


def result_digest(result: dict[str, Any]) -> str:
    return hashlib.sha256(json.dumps(result, sort_keys=True, separators=(",", ":"),
                                    ensure_ascii=True, allow_nan=False).encode()).hexdigest()


def _cell(value: Any) -> str:
    return html.escape(str(value), quote=True).replace("\\", "\\\\").replace(
        "|", "\\|").replace(chr(96), "\\" + chr(96)).replace("*", "\\*")


def summary_lines(result: dict[str, Any]) -> list[str]:
    """Witness the JSON digest and show the account arithmetic and movement references."""
    lines = [
        f"- Result: {result['status']}; currency: {_cell(result['currency'] or 'unconfirmed')}.",
        f"- Currency evidence: {_cell(result['currency_evidence'] or 'missing')}.",
        f"- Result digest: {result_digest(result)}", "",
        "| Account | Opening | Movement | Expected close | Actual close | Unexplained |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in result["accounts"]:
        lines.append("| " + " | ".join(_cell(row[key]) for key in ("account_id", *_NUMBERS)) + " |")
    if result["total"] is not None:
        lines.append("| Total | " + " | ".join(_cell(result["total"][key]) for key in _NUMBERS) + " |")
    lines += ["", "| Movement | Account | Date | Debit | Credit | Description | Evidence |",
              "| --- | --- | --- | ---: | ---: | --- | --- |"]
    for row in result["movements"]:
        lines.append("| " + " | ".join(_cell(row[key]) for key in (
            "movement_id", "account_id", "date", "debit", "credit", "description",
            "evidence_reference")) + " |")
    lines.extend("- Issue: " + _cell(issue) for issue in result["issues"])
    return lines


def validate_result(value: Any, sources: dict, exceptions: list) -> dict[str, Any]:
    """Check shape and ties to the other pack members before displaying the evidence."""
    if (not isinstance(value, dict) or set(value) != _RESULT_FIELDS
            or value["schema_version"] != 1 or type(value["schema_version"]) is not int
            or value["basis"] != "ledger_equity" or not isinstance(value["status"], str)
            or value["status"] not in {"PASS", "REVIEW", "BLOCKED"}
            or type(value["complete"]) is not bool or not isinstance(value["issues"], list)
            or not all(isinstance(item, str) for item in value["issues"])
            or not isinstance(value["accounts"], list) or not isinstance(value["movements"], list)):
        raise ControlInputError("Malformed equity reconciliation result.")
    for field in ("tenant", "opening_date", "closing_date", "currency", "currency_evidence",
                  "schedule_sha256", "tolerance"):
        if not isinstance(value[field], str):
            raise ControlInputError("Equity result fields must be strings.")
    if sources.get("equity_schedule") != value["schedule_sha256"]:
        raise ControlInputError("Equity schedule provenance disagrees with the pack.")
    for row in value["accounts"]:
        if not isinstance(row, dict) or set(row) != {"account_id", "movement_ids", *_NUMBERS}:
            raise ControlInputError("Malformed equity account evidence.")
        if not isinstance(row["account_id"], str) or not isinstance(row["movement_ids"], list):
            raise ControlInputError("Malformed equity account identity.")
        if any(not isinstance(row[key], str) for key in _NUMBERS):
            raise ControlInputError("Equity account amounts must be decimal strings.")
    if value["total"] is not None and (
        not isinstance(value["total"], dict) or set(value["total"]) != set(_NUMBERS)
        or any(not isinstance(item, str) for item in value["total"].values())
    ):
        raise ControlInputError("Malformed equity total evidence.")
    for row in value["movements"]:
        if not isinstance(row, dict) or set(row) != _MOVEMENT_FIELDS:
            raise ControlInputError("Malformed equity movement evidence.")
        if any(not isinstance(item, str) for item in row.values()):
            raise ControlInputError("Equity movement fields must be strings.")
    tolerance = _amount(value["tolerance"], "tolerance")
    if value["status"] == "BLOCKED":
        if value["accounts"] or value["total"] is not None or not value["issues"]:
            raise ControlInputError("Blocked equity evidence must omit calculated balances.")
    else:
        if not value["accounts"] or value["total"] is None or value["issues"] or not value["complete"]:
            raise ControlInputError("Calculated equity evidence requires complete accounts and totals.")
        try:
            with localcontext(Context(prec=28)) as context:
                context.traps[Inexact] = True
                totals = {key: Decimal(0) for key in _NUMBERS}
                accounts: set[str] = set()
                movements: set[str] = set()
                needs_review = False
                for row in value["movements"]:
                    identifier = _text(row["movement_id"], "movement ID")
                    if identifier in movements:
                        raise ControlInputError("Duplicate equity movement evidence.")
                    movements.add(identifier)
                    debit, credit = _amount(row["debit"], "debit"), _amount(row["credit"], "credit")
                    if (debit > 0) == (credit > 0):
                        raise ControlInputError("Equity movement must have exactly one positive side.")
                for row in value["accounts"]:
                    identifier = _text(row["account_id"], "account ID")
                    if identifier in accounts:
                        raise ControlInputError("Duplicate equity account evidence.")
                    accounts.add(identifier)
                    numbers = {}
                    for key in _NUMBERS:
                        raw = row[key]
                        numbers[key] = -_amount(raw[1:], key) if raw.startswith("-") else _amount(raw, key)
                    linked = [item for item in value["movements"] if item["account_id"] == identifier]
                    movement = sum((Decimal(item["credit"]) - Decimal(item["debit"]) for item in linked), Decimal(0))
                    if (row["movement_ids"] != [item["movement_id"] for item in linked]
                            or numbers["movement"] != movement
                            or numbers["expected_close"] != numbers["opening"] + movement
                            or numbers["unexplained"] != numbers["actual_close"] - numbers["expected_close"]):
                        raise ControlInputError("Equity account arithmetic or movement links disagree.")
                    needs_review |= abs(numbers["unexplained"]) > tolerance
                    for key in _NUMBERS:
                        totals[key] += numbers[key]
                if any(row["account_id"] not in accounts for row in value["movements"]):
                    raise ControlInputError("Equity movement references an unknown account.")
                for key, total in totals.items():
                    raw = value["total"][key]
                    actual = -_amount(raw[1:], key) if raw.startswith("-") else _amount(raw, key)
                    if actual != total:
                        raise ControlInputError("Equity totals disagree with the account evidence.")
                needs_review |= abs(totals["unexplained"]) > tolerance
                if value["status"] != ("REVIEW" if needs_review else "PASS"):
                    raise ControlInputError("Equity status disagrees with its arithmetic.")
        except (Inexact, InvalidOperation) as exc:
            raise ControlInputError("Equity evidence exceeds supported exact decimal precision.") from exc
    statuses = {row["status"] for row in exceptions if row["control"] == "equity_reconciliation"}
    expected = "BLOCKED" if "BLOCKED" in statuses else "REVIEW" if "REVIEW" in statuses else "PASS"
    if value["status"] != expected:
        raise ControlInputError("Equity result status disagrees with its findings.")
    return value
