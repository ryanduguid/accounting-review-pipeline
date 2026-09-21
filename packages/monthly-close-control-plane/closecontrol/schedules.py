"""Review explicitly mapped schedules without posting or approving accounting."""
from __future__ import annotations

import json
import re
from collections import defaultdict
from datetime import date
from decimal import Decimal, Inexact, localcontext
from pathlib import Path
from typing import Any

from .errors import ControlInputError
from .loader import SourceSnapshot


def _object(value: Any, fields: str, label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != set(fields.split()):
        raise ControlInputError(f"{label}: expected exactly {fields}.")
    return value


def _text(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip() or any(ord(ch) < 32 for ch in value):
        raise ControlInputError(f"{label}: expected non-blank text without outer spaces or controls.")
    return value


def _money(value: Any, label: str) -> Decimal:
    if not isinstance(value, str) or not re.fullmatch(r"-?\d{1,16}(?:\.\d{1,2})?", value):
        raise ControlInputError(f"{label}: expected a signed decimal string with at most 2 places.")
    return Decimal(value)


def _date(value: Any, label: str) -> date:
    try:
        result = date.fromisoformat(value)
    except (ValueError, TypeError) as exc:
        raise ControlInputError(f"{label}: expected YYYY-MM-DD.") from exc
    if result.isoformat() != value:
        raise ControlInputError(f"{label}: expected YYYY-MM-DD.")
    return result


def _rows(value: Any, fields: str, key: str, label: str) -> list[dict[str, Any]]:
    if not isinstance(value, list):
        raise ControlInputError(f"{label}: expected an array.")
    seen: set[str] = set()
    for row in value:
        _object(row, fields, label)
        identifier = _text(row[key], f"{label}.{key}")
        if identifier in seen:
            raise ControlInputError(f"{label}: duplicate {key} {identifier}.")
        seen.add(identifier)
    return value


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ControlInputError(f"Duplicate JSON field: {key}.")
        result[key] = value
    return result


def review_schedule(path: Path, *, kind: str) -> dict[str, Any]:
    """Read one evidence snapshot and return arithmetic, findings and provenance."""
    snapshot = SourceSnapshot.capture(path, label="schedule")
    try:
        data = json.loads(snapshot.text(label="schedule", encoding="utf-8-sig"), object_pairs_hook=_pairs)
    except (ValueError, RecursionError) as exc:
        raise ControlInputError(f"Invalid schedule JSON: {exc}.") from exc
    functions = {"expenses": _expenses, "migration": _migration, "interentity": _interentity}
    if kind not in functions:
        raise ControlInputError("Unknown schedule kind.")
    if not isinstance(data, dict) or type(data.get("schema_version")) is not int or data["schema_version"] != 1:
        raise ControlInputError("schema_version must be integer 1.")
    currency = data.get("currency")
    if not isinstance(currency, str) or not re.fullmatch("[A-Z]{3}", currency):
        raise ControlInputError("currency must be an explicit 3-letter code.")
    _date(data.get("cutoff"), "cutoff")
    with localcontext() as context:
        context.prec = 80
        context.traps[Inexact] = True
        result = functions[kind](data)
    return {"schema_version": 1, "kind": kind, "currency": currency,
            "cutoff": data["cutoff"], "source_sha256": snapshot.sha256,
            "status": "REVIEW" if result["findings"] else "PASS",
            "scope": "Supplied records and explicit mappings only; no posting or approval.",
            **result}


def _expenses(data: dict[str, Any]) -> dict[str, Any]:
    _object(data, "schema_version tenant currency cutoff invoices claims payments ledger_balance", "expenses")
    tenant = _text(data["tenant"], "tenant")
    invoices = _rows(data["invoices"], "invoice_id supplier reference amount evidence", "invoice_id", "invoices")
    if not invoices:
        raise ControlInputError("Supply the invoice population; empty evidence cannot establish completeness.")
    claims = _rows(data["claims"], "claim_id invoice_id employee amount approved", "claim_id", "claims")
    payments = _rows(data["payments"], "payment_id claim_id amount date reference", "payment_id", "payments")
    findings: list[dict[str, str]] = []
    amounts: dict[str, Decimal] = {}
    allocated: dict[str, Decimal] = defaultdict(Decimal)
    paid: dict[str, Decimal] = defaultdict(Decimal)
    refs: dict[tuple[str, str], list[str]] = defaultdict(list)
    for row in invoices:
        amounts[row["invoice_id"]] = _money(row["amount"], "invoice amount")
        if amounts[row["invoice_id"]] <= 0:
            raise ControlInputError("Invoice amount must be positive.")
        refs[(_text(row["supplier"], "supplier"), _text(row["reference"], "reference"))].append(row["invoice_id"])
        if not isinstance(row["evidence"], str):
            raise ControlInputError("evidence must be text; use an empty string for missing evidence.")
        if not row["evidence"].strip():
            findings.append({"code": "MISSING_RECEIPT", "id": row["invoice_id"]})
    for ids in refs.values():
        if len(ids) > 1:
            findings.append({"code": "DUPLICATE_INVOICE_REFERENCE", "id": ", ".join(ids)})
    for row in claims:
        _text(row["employee"], "employee")
        _text(row["invoice_id"], "invoice_id")
        amount = _money(row["amount"], "claim amount")
        if amount <= 0 or type(row["approved"]) is not bool:
            raise ControlInputError("Claims require a positive amount and boolean approved.")
        allocated[row["invoice_id"]] += amount
        if row["invoice_id"] not in amounts:
            findings.append({"code": "UNKNOWN_INVOICE", "id": row["claim_id"]})
        if not row["approved"]:
            findings.append({"code": "UNAPPROVED_CLAIM", "id": row["claim_id"]})
    for identifier, amount in amounts.items():
        difference = amount - allocated[identifier]
        if difference:
            findings.append({"code": "OVER_ALLOCATED" if difference < 0 else "UNALLOCATED_INVOICE",
                             "id": identifier, "difference": str(difference)})
    claim_ids = {row["claim_id"] for row in claims}
    for row in payments:
        _text(row["claim_id"], "payment claim_id")
        _text(row["reference"], "payment reference")
        amount = _money(row["amount"], "payment amount")
        if amount <= 0 or _date(row["date"], "payment date") > _date(data["cutoff"], "cutoff"):
            raise ControlInputError("Payments require a positive amount and date on or before cutoff.")
        paid[row["claim_id"]] += amount
        if row["claim_id"] not in claim_ids:
            findings.append({"code": "UNLINKED_PAYMENT", "id": row["payment_id"], "amount": str(amount)})
    workpaper = []
    for row in claims:
        remaining = _money(row["amount"], "claim amount") - paid[row["claim_id"]]
        workpaper.append({**row, "paid": str(paid[row["claim_id"]]), "remaining": str(remaining)})
        if remaining < 0 or (remaining > 0 and row["approved"]):
            findings.append({"code": "OVERPAID" if remaining < 0 else "UNPAID_APPROVED",
                             "id": row["claim_id"], "amount": str(remaining)})
    # Orphan payments still reduce the clearing account.
    expected = sum((_money(row["amount"], "claim amount") for row in claims), Decimal(0)) - sum(paid.values(), Decimal(0))
    difference = _money(data["ledger_balance"], "ledger_balance") - expected
    if difference:
        findings.append({"code": "CLEARING_DIFFERENCE", "id": tenant, "difference": str(difference)})
    return {"tenant": tenant, "claims": workpaper, "payments": payments,
            "invoices": invoices, "expected_ledger_balance": str(expected),
            "ledger_balance": data["ledger_balance"], "ledger_difference": str(difference), "findings": findings}


def _migration(data: dict[str, Any]) -> dict[str, Any]:
    _object(data, "schema_version tenant currency cutoff mapping before after", "migration")
    tenant = _text(data["tenant"], "tenant")
    mappings = _rows(data["mapping"], "mapping_id source_account target_account weight", "mapping_id", "mapping")
    before = _rows(data["before"], "row_id item_id account amount reference tax_code", "row_id", "before")
    after = _rows(data["after"], "row_id item_id account amount reference tax_code", "row_id", "after")
    if not before:
        raise ControlInputError("Supply the pre-migration population.")
    weights: dict[str, list[tuple[str, Decimal]]] = defaultdict(list)
    pairs: set[tuple[str, str]] = set()
    for row in mappings:
        source = _text(row["source_account"], "source_account")
        target = _text(row["target_account"], "target_account")
        if not isinstance(row["weight"], str) or not re.fullmatch(r"(?:0(?:\.\d{1,6})?|1(?:\.0{1,6})?)", row["weight"]):
            raise ControlInputError("Mapping weight must be a decimal string in (0, 1].")
        weight = Decimal(row["weight"])
        if weight <= 0 or (source, target) in pairs:
            raise ControlInputError("Mapping weights must be positive and source/target pairs unique.")
        pairs.add((source, target))
        weights[source].append((target, weight))
    if any(sum((weight for _, weight in rows), Decimal(0)) != 1 for rows in weights.values()):
        raise ControlInputError("Each source account's mapping weights must sum to 1.")
    expected: dict[tuple[str, str, str, str], Decimal] = defaultdict(Decimal)
    actual: dict[tuple[str, str, str, str], Decimal] = defaultdict(Decimal)
    findings: list[dict[str, str]] = []
    for rows, target_values, source_side in ((before, expected, True), (after, actual, False)):
        seen_keys: set[tuple[str, str]] = set()
        for row in rows:
            for field in ("item_id", "account", "reference", "tax_code"):
                _text(row[field], field)
            amount = _money(row["amount"], "amount")
            identity = (row["item_id"], row["account"])
            if identity in seen_keys:
                findings.append({"code": "DUPLICATE_ITEM", "id": row["row_id"], "side": "before" if source_side else "after"})
            seen_keys.add(identity)
            destinations = weights.get(row["account"], []) if source_side else [(row["account"], Decimal(1))]
            if not destinations:
                findings.append({"code": "UNMAPPED_ACCOUNT", "id": row["account"]})
            for account, weight in destinations:
                target_values[(row["item_id"], account, row["reference"], row["tax_code"])] += amount * weight
    differences = []
    for key in sorted(set(expected) | set(actual)):
        difference = actual.get(key, Decimal(0)) - expected.get(key, Decimal(0))
        if key not in expected or key not in actual or difference:
            differences.append({"item_id": key[0], "account": key[1], "reference": key[2],
                                "tax_code": key[3], "expected": str(expected.get(key, Decimal(0))),
                                "actual": str(actual.get(key, Decimal(0))), "difference": str(difference),
                                "presence": "both" if key in expected and key in actual else "before" if key in expected else "after"})
    if differences:
        findings.append({"code": "ITEM_DIFFERENCES", "id": str(len(differences))})
    return {"tenant": tenant, "mapping": mappings, "differences": differences,
            "before_total": str(sum((_money(row["amount"], "amount") for row in before), Decimal(0))),
            "after_total": str(sum(actual.values(), Decimal(0))), "findings": findings,
            "policy": "Item IDs, references and tax codes must be preserved exactly. Weights allocate amounts without rounding."}


def _interentity(data: dict[str, Any]) -> dict[str, Any]:
    _object(data, "schema_version currency cutoff entities records", "interentity")
    if not isinstance(data["entities"], list) or len(data["entities"]) < 2:
        raise ControlInputError("Supply at least 2 explicit entity IDs.")
    entities = [_text(value, "entity") for value in data["entities"]]
    if len(set(entities)) != len(entities):
        raise ControlInputError("Entity IDs must be unique.")
    records = _rows(data["records"], "row_id entity counterparty account reference date amount disputed evidence", "row_id", "records")
    if not records:
        raise ControlInputError("Supply both entities' records; empty evidence cannot establish agreement.")
    groups: dict[tuple[str, str, str], dict[str, list[dict[str, Any]]]] = {}
    findings = []
    cutoff = _date(data["cutoff"], "cutoff")
    for row in records:
        for field in ("entity", "counterparty", "account", "reference", "evidence"):
            _text(row[field], field)
        if row["entity"] not in entities or row["counterparty"] not in entities or row["entity"] == row["counterparty"]:
            raise ControlInputError("Each record requires 2 distinct declared entity IDs.")
        _money(row["amount"], "amount")
        if _date(row["date"], "date") > cutoff or type(row["disputed"]) is not bool:
            raise ControlInputError("Records require a date on or before cutoff and boolean disputed.")
        left, right = sorted((row["entity"], row["counterparty"]))
        group = groups.setdefault((left, right, row["reference"]), {left: [], right: []})
        group[row["entity"]].append(row)
        if row["disputed"]:
            findings.append({"code": "DISPUTED", "id": row["row_id"]})
    workpaper = []
    for (left, right, reference), group in sorted(groups.items()):
        a = sum((_money(row["amount"], "amount") for row in group[left]), Decimal(0))
        b = sum((_money(row["amount"], "amount") for row in group[right]), Decimal(0))
        difference = a + b
        status = "UNMATCHED" if not group[left] or not group[right] else "DIFFERENCE" if difference else "AGREES"
        if status != "AGREES":
            findings.append({"code": status, "id": f"{left}/{right}/{reference}", "difference": str(difference)})
        workpaper.append({"entity": left, "counterparty": right, "reference": reference,
                          "entity_balance": str(a), "counterparty_balance": str(b),
                          "difference": str(difference), "status": status, "records": group})
    return {"entities": entities, "pairs": workpaper, "findings": findings,
            "policy": "Debit-positive balances; exact reference matching only. No inferred identities or elimination entries."}
