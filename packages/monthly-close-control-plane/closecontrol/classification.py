"""Compare supplied posting assignments with separately supplied bill-purpose evidence."""
from __future__ import annotations

import csv
import io
import json
from collections import Counter
from datetime import date
from decimal import Context, Decimal, localcontext
from pathlib import Path
from typing import Any

from .drivers import _transactions
from .errors import ControlInputError
from .loader import SourceSnapshot, _text, parse_iso_date
from .reconciliation import COLUMNS
from .viewer import _no_duplicate_keys

BOUNDARY = (
    "Supplied transaction selection and coding assertions only. References are labels, "
    "not authenticated bills or proof of appropriate accounting treatment. Agreement does "
    "not establish completeness, approve a correction or change any close finding or status."
)
FIELDS = {"original_account_id", "transaction_id", "current_account_id",
          "expected_account_id", "purpose", "evidence_reference"}


def classify(original: Path, current: Path, evidence: Path, *, currency: str) -> dict[str, Any]:
    """Retain original discrepancies and check the explicitly mapped current assignments."""
    # Each distinct path is captured once, including the initial same-file comparison.
    snapshots = {path: SourceSnapshot.capture(path, label="Classification source")
                 for path in dict.fromkeys(path.resolve() for path in (original, current, evidence))}
    before, after, assertions = (snapshots[path.resolve()] for path in (original, current, evidence))
    try:
        data = json.loads(assertions.text(label="Coding evidence", encoding="utf-8-sig"),
                          object_pairs_hook=_no_duplicate_keys)
    except (ValueError, RecursionError) as exc:
        raise ControlInputError(f"Invalid coding evidence JSON: {exc}") from exc
    fields = {"schema_version", "tenant", "currency", "period_start", "period_end",
              "original_sha256", "current_sha256", "expectations"}
    if (not isinstance(data, dict) or set(data) != fields
            or type(data["schema_version"]) is not int or data["schema_version"] != 1
            or data["original_sha256"] != before.sha256 or data["current_sha256"] != after.sha256
            or not isinstance(data["expectations"], list)):
        raise ControlInputError("Coding evidence must use schema 1 and bind both transaction digests.")
    if (not isinstance(currency, str) or len(currency) != 3
            or not currency.isascii() or not currency.isupper() or not currency.isalpha()
            or data["currency"] != currency):
        raise ControlInputError("Declare the matching three-letter currency explicitly.")
    for field in ("tenant", "period_start", "period_end"):
        if not isinstance(data[field], str):
            raise ControlInputError(f"{field} must be text.")
        data[field] = _text(data[field], field=field, path=assertions.path, row_number=0)
    start, end = (parse_iso_date(data[field], field=field, path=assertions.path)
                  for field in ("period_start", "period_end"))
    if start > end:
        raise ControlInputError("period_start must not follow period_end.")
    with localcontext(Context(prec=40)):
        originals = _selection(before, currency, data["tenant"], start, end)
        currents = _selection(after, currency, data["tenant"], start, end)
    policies = {}
    for number, row in enumerate(data["expectations"], start=1):
        if not isinstance(row, dict) or set(row) != FIELDS:
            raise ControlInputError("Each coding assertion needs exactly the documented fields.")
        if any(not isinstance(value, str) for value in row.values()):
            raise ControlInputError("Coding assertion fields must be text.")
        row = {key: _text(value, field=key, path=assertions.path, row_number=number,
                          allow_empty=key in {"expected_account_id", "purpose", "evidence_reference"})
               for key, value in row.items()}
        key = (row["original_account_id"], row["transaction_id"])
        if key in policies or key not in originals:
            raise ControlInputError("Coding assertion key is duplicated or absent from the original selection.")
        policies[key] = row
    targets = {key: (policies[key]["current_account_id"], key[1]) if key in policies else key
               for key in originals}
    target_counts = Counter(targets.values())
    used: set[tuple[str, str]] = set()
    items = []
    for key, old in sorted(originals.items()):
        policy = policies.get(key)
        target = targets[key]
        new = currents.get(target)
        reasons = []
        expected = (policy["expected_account_id"] if policy is not None and all(policy[field] for field in
                    ("expected_account_id", "purpose", "evidence_reference")) else None)
        if expected is None:
            reasons.append("MISSING_CLASSIFICATION_EVIDENCE")
        if new is None:
            reasons.append("CURRENT_TRANSACTION_NOT_MATCHED")
        else:
            used.add(target)
            if target_counts[target] > 1:
                reasons.append("CURRENT_TRANSACTION_REUSED")
            if (old["Date"] != new["Date"] or old["Reference"] != new["Reference"]
                    or old["Description"] != new["Description"] or old["amount"] != new["amount"]):
                reasons.append("TRANSACTION_FACTS_CHANGED")
            if expected is not None and new["AccountID"] != expected:
                reasons.append("CODING_DIFFERENCE")
        items.append({
            "original": _posting(old), "current": _posting(new) if new is not None else None,
            "evidence": policy,
            "original_disagrees_with_evidence": (
                old["AccountID"] != expected if expected is not None else None),
            "status": "REVIEW" if reasons else "PASS", "reasons": reasons,
        })
    unmatched = [_posting(row) for key, row in sorted(currents.items()) if key not in used]
    return {
        "schema_version": 1, "status": "PASS" if items and not unmatched
        and all(item["status"] == "PASS" for item in items) else "REVIEW",
        "tenant": data["tenant"], "currency": currency,
        "period_start": start.isoformat(), "period_end": end.isoformat(),
        "scope": "The same supplied transaction selection before and after possible recoding.",
        "boundary": BOUNDARY, "original_count": len(originals), "current_count": len(currents),
        "source_sha256": {"original": before.sha256, "current": after.sha256,
                          "coding_evidence": assertions.sha256},
        "items": items, "unmatched_current": unmatched,
    }


def _selection(source: SourceSnapshot, currency: str, tenant: str, start: date, end: date
               ) -> dict[tuple[str, str], dict[str, Any]]:
    result = {}
    try:
        reader = csv.DictReader(io.StringIO(source.text(label="Transactions", encoding="utf-8-sig")),
                                strict=True)
        if (not reader.fieldnames or len(reader.fieldnames) != len(COLUMNS)
                or set(reader.fieldnames) != set(COLUMNS)):
            raise ControlInputError("Classification transactions need the exact canonical columns.")
        for values in reader:
            if set(values) != set(COLUMNS) or any(value is None for value in values.values()):
                raise ControlInputError("Classification transaction rows need every explicit cell.")
    except csv.Error as exc:
        raise ControlInputError(f"Malformed classification transaction CSV: {exc}") from exc
    for (owner, account), rows in _transactions(source, currency).items():
        if owner != tenant:
            raise ControlInputError("Transactions must belong to the declared tenant.")
        for row in rows:
            if not start <= row["date"] <= end:
                raise ControlInputError("Every transaction must fall within the declared selection period.")
            result[(account, row["TransactionID"])] = row
    return result


def _posting(row: dict[str, Any]) -> dict[str, str]:
    return {key: row[key] for key in ("Tenant", "AccountID", "Currency", "TransactionID", "Date",
                                     "Reference", "Description")} | {
        "Debit": f"{row['amount'] if row['amount'] > 0 else Decimal(0):.2f}",
        "Credit": f"{row['amount'].copy_abs() if row['amount'] < 0 else Decimal(0):.2f}"}
