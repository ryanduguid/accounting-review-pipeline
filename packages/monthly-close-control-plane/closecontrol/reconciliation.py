"""Local transaction matching; a suggested allocation never clears an item."""
from __future__ import annotations

import csv
import json
import re
from collections import defaultdict
from datetime import date, timedelta
from decimal import Context, Decimal, localcontext
from pathlib import Path

from .errors import ControlInputError
from .loader import SourceSnapshot, _read_csv_rows, _text, parse_money

COLUMNS = ("Tenant", "AccountID", "Currency", "TransactionID", "Date", "Reference",
           "Description", "Debit", "Credit")
DECISION_COLUMNS = ("Group", "TransactionID", "Decision", "Note")
IDENTITY_COLUMNS = ("Tenant", "AccountID", "Currency")
BOUNDARY = "Review evidence only. Matching records an allocation; it does not approve a close."


def _date(value: str) -> date:
    if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
        raise ControlInputError("Dates must use YYYY-MM-DD.")
    return date.fromisoformat(value)


def _money(value: str, *, field: str = "amount", path: Path = Path("configuration"),
           row: int = 0) -> Decimal:
    amount = parse_money(value, field=field, path=path, row_number=row)
    if abs(amount) >= Decimal("1000000000000000") or amount != amount.quantize(Decimal("0.01")):
        raise ControlInputError(f"{path}: {field} must be below 10^15 with at most two decimal places.")
    return amount


def _item(values: dict, identity: dict[str, str], path: Path, row: int) -> dict[str, str]:
    if set(values) != set(COLUMNS) or any(not isinstance(v, str) for v in values.values()):
        raise ControlInputError(f"{path}: row {row} must contain every transaction field as text.")
    item = {key: _text(values[key], field=key, path=path, row_number=row,
                      allow_empty=key in {"Reference", "Description"}) for key in COLUMNS}
    if any(item[key] != identity[key] for key in IDENTITY_COLUMNS):
        raise ControlInputError(f"{path}: row {row} belongs to a different tenant, account or currency.")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,79}", item["TransactionID"]):
        raise ControlInputError(f"{path}: row {row} has an invalid TransactionID.")
    _date(item["Date"])
    debit = _money(item["Debit"], field="Debit", path=path, row=row)
    credit = _money(item["Credit"], field="Credit", path=path, row=row)
    if debit < 0 or credit < 0 or (debit > 0) == (credit > 0):
        raise ControlInputError(f"{path}: row {row} needs exactly one positive Debit or Credit.")
    item.update(Debit=f"{debit:.2f}", Credit=f"{credit:.2f}")
    return item


def _total(items: list[dict[str, str]]) -> Decimal:
    return sum((Decimal(item["Debit"]) - Decimal(item["Credit"]) for item in items), Decimal(0))


def _opening(snapshot: SourceSnapshot, identity: dict[str, str], start: date
             ) -> tuple[list[dict[str, str]], dict[str, str]]:
    payload = json.loads(snapshot.text(label="Opening items", encoding="utf-8-sig"))
    fields = {"schema", "identity", "period_end", "balance", "items", "notes"}
    if (not isinstance(payload, dict) or set(payload) != fields
            or payload["schema"] != "clearing-carry-v1" or payload["identity"] != identity
            or not isinstance(payload["period_end"], str)
            or not isinstance(payload["balance"], str)
            or not isinstance(payload["items"], list) or not isinstance(payload["notes"], dict)):
        raise ControlInputError("Opening items have an invalid schema or a different account identity.")
    previous_end = _date(payload["period_end"])
    if start - previous_end != timedelta(days=1):
        raise ControlInputError("Opening items must come from the immediately preceding period.")
    items = []
    for index, value in enumerate(payload["items"], 1):
        if not isinstance(value, dict):
            raise ControlInputError("Opening items must be transaction objects.")
        item = _item(value, identity, snapshot.path, index)
        if _date(item["Date"]) > previous_end:
            raise ControlInputError("Opening item dates cannot be later than their period end.")
        items.append(item)
    if _total(items) != _money(payload["balance"]):
        raise ControlInputError("Opening items do not equal their recorded balance.")
    ids = {item["TransactionID"] for item in items}
    notes = payload["notes"]
    if not set(notes).issubset(ids) or any(not isinstance(v, str) for v in notes.values()):
        raise ControlInputError("Opening review notes must name outstanding transaction IDs.")
    notes = {key: _text(value, field="Note", path=snapshot.path, row_number=0)
             for key, value in notes.items()}
    return items, notes


def _decisions(snapshot: SourceSnapshot | None, items: dict[str, dict[str, str]]) -> list[dict]:
    if snapshot is None:
        return []
    groups: dict[str, list[dict[str, str]]] = defaultdict(list)
    seen: set[str] = set()
    for row, values in _read_csv_rows(snapshot, DECISION_COLUMNS, label="Decisions"):
        if any(value is None for value in values.values()):
            raise ControlInputError("Every decisions row must contain all four fields.")
        record = {key: _text(values[key], field=key, path=snapshot.path, row_number=row,
                            allow_empty=key in {"Decision", "Note"}) for key in DECISION_COLUMNS}
        if record["Decision"] not in {"", "accept", "reject"}:
            raise ControlInputError("Decision must be blank, accept or reject.")
        item_id = record["TransactionID"]
        if item_id not in items or item_id in seen:
            raise ControlInputError("Decisions contain an unknown or repeated transaction ID.")
        seen.add(item_id)
        groups[record["Group"]].append(record)
    result = []
    for group, records in sorted(groups.items()):
        decision, note = records[0]["Decision"], records[0]["Note"]
        if len(records) < 2 or any((r["Decision"], r["Note"]) != (decision, note) for r in records):
            raise ControlInputError("Each group needs at least two items with the same decision and note.")
        if decision and not note:
            raise ControlInputError("Accepted and rejected groups need a review note.")
        ids = sorted((r["TransactionID"] for r in records),
                     key=lambda key: (items[key]["Date"], key))
        if decision == "accept" and _total([items[key] for key in ids]) != 0:
            raise ControlInputError(f"Accepted group {group!r} does not sum to zero.")
        result.append({"group": group, "ids": ids, "decision": decision, "note": note})
    return result


def reconcile(transactions: Path, *, tenant: str, account_id: str, currency: str,
              period_start: str, period_end: str, opening_balance: str, closing_balance: str,
              opening_items: Path | None = None, decisions: Path | None = None) -> dict:
    # Input bounds and a private context keep sums exact even if the caller changes Decimal settings.
    with localcontext(Context(prec=40)):
        return _reconcile(transactions, tenant, account_id, currency, period_start, period_end,
                          opening_balance, closing_balance, opening_items, decisions)


def _reconcile(transactions: Path, tenant: str, account_id: str, currency: str,
               period_start: str, period_end: str, opening_balance: str, closing_balance: str,
               opening_items: Path | None, decisions: Path | None) -> dict:
    identity = {key: _text(value, field=key, path=Path("configuration"), row_number=0)
                for key, value in zip(IDENTITY_COLUMNS, (tenant, account_id, currency))}
    if not re.fullmatch(r"[A-Z]{3}", identity["Currency"]):
        raise ControlInputError("Currency must be a three-letter uppercase code; no conversion is performed.")
    start, end = _date(period_start), _date(period_end)
    if start > end:
        raise ControlInputError("Period start cannot be after period end.")
    opening, closing = _money(opening_balance), _money(closing_balance)
    snapshots = {"transactions": SourceSnapshot.capture(transactions, label="Transactions")}
    if opening_items is not None:
        snapshots["opening_items"] = SourceSnapshot.capture(opening_items, label="Opening items")
    if decisions is not None:
        snapshots["decisions"] = SourceSnapshot.capture(decisions, label="Decisions")
    old, notes = _opening(snapshots["opening_items"], identity, start) if opening_items else ([], {})
    current = []
    try:
        for row, values in _read_csv_rows(snapshots["transactions"], COLUMNS, label="Transactions"):
            item = _item(values, identity, transactions, row)
            if not start <= _date(item["Date"]) <= end:
                raise ControlInputError(f"{transactions}: row {row} falls outside the reporting period.")
            current.append(item)
    except csv.Error as exc:
        raise ControlInputError(f"Invalid transaction CSV: {exc}") from exc
    ordered = sorted(old + current, key=lambda item: (item["Date"], item["TransactionID"]))
    if len(ordered) > 100000:
        raise ControlInputError("A reconciliation supports at most 100,000 outstanding and current items.")
    by_id = {item["TransactionID"]: item for item in ordered}
    if len(by_id) != len(ordered):
        raise ControlInputError("Repeated TransactionID within or across opening and current items.")
    allocations = _decisions(snapshots.get("decisions"), by_id)
    accepted = {key for group in allocations if group["decision"] == "accept" for key in group["ids"]}
    grouped = {key for group in allocations for key in group["ids"]}
    for group in allocations:
        if group["note"]:
            notes.update({key: group["note"] for key in group["ids"]})
    outstanding = [item for item in ordered if item["TransactionID"] not in accepted]
    references: dict[str, list[dict[str, str]]] = defaultdict(list)
    for item in outstanding:
        if item["Reference"] and item["TransactionID"] not in grouped:
            references[item["Reference"]].append(item)
    suggestions: list[dict] = []
    # ponytail: suggest whole reference groups only; manual groups cover partial and cross-reference matches.
    for reference, items in sorted(references.items()):
        if 2 <= len(items) <= 20 and _total(items) == 0:
            suggestions.append({"group": f"S{len(suggestions) + 1}",
                                "ids": [item["TransactionID"] for item in items],
                                "reason": f"Shared reference {reference!r}; total 0.00."})
    movement = _total(current)
    opening_difference = _total(old) - opening
    closing_difference = opening + movement - closing
    status = "BLOCKED" if opening_difference or closing_difference else "REVIEW" if outstanding else "PASS"
    return {"schema": "clearing-reconciliation-v1", "status": status, "identity": identity,
            "period_start": period_start, "period_end": period_end,
            "opening_balance": f"{opening:.2f}", "closing_balance": f"{closing:.2f}",
            "movement": f"{movement:.2f}", "opening_difference": f"{opening_difference:.2f}",
            "closing_difference": f"{closing_difference:.2f}",
            "outstanding_total": f"{_total(outstanding):.2f}", "transactions": ordered,
            "opening_ids": [item["TransactionID"] for item in old],
            "outstanding": outstanding, "suggestions": suggestions, "decisions": allocations,
            "notes": notes, "review_boundary": BOUNDARY,
            "source_sha256": {key: {"file": value.path.name, "sha256": value.sha256}
                              for key, value in snapshots.items()}}
