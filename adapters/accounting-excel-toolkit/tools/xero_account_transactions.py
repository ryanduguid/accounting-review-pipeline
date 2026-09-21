"""Observed Xero Account Transactions CSV conversion with explicit line identity."""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import re
import unicodedata
from datetime import datetime
from decimal import Decimal, localcontext
from pathlib import Path

HEADER = ["Date", "Source", "Description", "Reference", "Debit", "Credit", "Running Balance", "Gross", "GST"]
OUTPUT = ["Tenant", "AccountID", "Currency", "TransactionID", "Date", "Reference", "Description", "Debit", "Credit"]
MAX_DETAIL_ROWS = 100_000


def _valid_text(value, *, field, allow_empty=False):
    if not isinstance(value, str) or (not value and not allow_empty):
        raise ValueError(f"{field} must be non-empty text.")
    if any(unicodedata.category(character) in {"Cc", "Cf", "Cs"} for character in value):
        raise ValueError(f"{field} contains a control or formatting character.")
    return value


def amount(text):
    if not re.fullmatch(r"(?:\d{1,15}|\d{1,3}(?:,\d{3}){1,4})(?:\.\d{1,2})?", text):
        raise ValueError(f"Expected an explicit non-negative amount with at most 2 places: {text!r}")
    return Decimal(text.replace(",", ""))


def inspect_export(content, *, tenant, account_name):
    """Select an exact account section; independently total detail, never formula caches."""
    rows = list(csv.reader(io.StringIO(content.decode("utf-8-sig"))))
    headers = [i for i, row in enumerate(rows) if row == HEADER]
    if len(headers) != 1:
        raise ValueError("Expected exactly one observed 9-column Account Transactions header.")
    header = headers[0]
    metadata = [row[0] for row in rows[:header] if row and row[0]]
    if "Account Transactions" not in metadata or tenant not in metadata:
        raise ValueError("Report title or tenant does not match supplied identity.")
    periods = [value for value in metadata if value.startswith("For the period ")]
    if len(periods) != 1:
        raise ValueError("Expected one explicit report period.")
    try:
        left, right = periods[0][15:].split(" to ")
        start = datetime.strptime(left, "%d %B %Y").date()
        end = datetime.strptime(right, "%d %B %Y").date()
    except ValueError as exc:
        raise ValueError("Expected 'For the period d Month yyyy to d Month yyyy'.") from exc
    if start > end:
        raise ValueError("Report period is reversed.")
    selected = False
    sections = 0
    complete = False
    opening_seen = False
    detail = []
    debit = credit = Decimal(0)
    with localcontext() as context:
        context.prec = 60
        for index, row in enumerate(rows[header + 1:], header + 2):
            if not row or not any(row):
                continue
            if len(row) != len(HEADER):
                raise ValueError(f"Row {index}: expected exactly 9 fields.")
            if row[0] == account_name and not any(row[1:]):
                selected = True
                sections += 1
                continue
            if not selected:
                continue
            if row[0] == "Opening Balance":
                if detail or opening_seen:
                    raise ValueError("Expected exactly one opening balance before transaction detail.")
                opening_seen = True
                continue
            if row[0] == "Total " + account_name:
                if not opening_seen:
                    raise ValueError("Selected account section is missing its opening balance.")
                if amount(row[4]) != debit or amount(row[5]) != credit:
                    raise ValueError("Account subtotal does not equal independently summed detail.")
                selected = False
                complete = True
                continue
            try:
                date_format = "%d/%m/%Y" if "/" in row[0] else "%d %b %Y"
                posted = datetime.strptime(row[0], date_format).date()
            except ValueError as exc:
                raise ValueError(f"Row {index}: unexpected row in selected account section.") from exc
            if not start <= posted <= end:
                raise ValueError(f"Row {index}: posting date is outside report period.")
            dr, cr = amount(row[4]), amount(row[5])
            if (dr > 0) == (cr > 0):
                raise ValueError(f"Row {index}: exactly one debit or credit must be positive.")
            debit += dr
            credit += cr
            if len(detail) >= MAX_DETAIL_ROWS:
                raise ValueError(f"Selected detail exceeds the {MAX_DETAIL_ROWS:,}-row clearing limit.")
            detail.append({"source_row": index, "Date": posted.isoformat(), "Source": row[1],
                           "Reference": row[3], "Description": row[2], "Debit": str(dr), "Credit": str(cr)})
    if sections != 1 or not complete or selected:
        raise ValueError("Expected exactly one complete selected account section.")
    return {"source_sha256": hashlib.sha256(content).hexdigest(), "tenant": tenant,
            "account_name": account_name, "period_start": start.isoformat(), "period_end": end.isoformat(),
            "debit": str(debit), "credit": str(credit), "detail": detail}


def convert(content, mapping, *, tenant, account_name, account_id, currency):
    result = inspect_export(content, tenant=tenant, account_name=account_name)
    _valid_text(tenant, field="Tenant")
    _valid_text(account_id, field="AccountID")
    _valid_text(currency, field="Currency")
    if not re.fullmatch("[A-Z]{3}", currency) or not account_id.strip():
        raise ValueError("Supply a 3-letter currency and explicit account ID.")
    if not isinstance(mapping, dict) or set(mapping) != {"source_sha256", "lines"} or mapping["source_sha256"] != result["source_sha256"]:
        raise ValueError("Line mapping must be bound to this exact source SHA-256.")
    lines = mapping["lines"]
    if not isinstance(lines, dict) or set(lines) != {str(row["source_row"]) for row in result["detail"]}:
        raise ValueError("Line mapping must cover every selected detail row exactly.")
    ids = list(lines.values())
    if any(not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,79}", value) for value in ids):
        raise ValueError("Mapping requires stable source-line IDs in the clearing schema.")
    if len(set(ids)) != len(ids):
        raise ValueError("Mapped source-line IDs must be unique.")
    output = []
    for row in result["detail"]:
        _valid_text(row["Reference"], field="Reference", allow_empty=True)
        _valid_text(row["Description"], field="Description", allow_empty=True)
        output.append({**{key: row[key] for key in ("Date", "Reference", "Description", "Debit", "Credit")},
                       "Tenant": tenant, "AccountID": account_id, "Currency": currency,
                       "TransactionID": lines[str(row["source_row"])]})
    return output


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    for field in ("tenant", "account-name"):
        parser.add_argument("--" + field, required=True)
    parser.add_argument("--mapping", type=Path)
    parser.add_argument("--account-id")
    parser.add_argument("--currency")
    args = parser.parse_args(argv)
    try:
        content = args.source.read_bytes()
        if args.mapping is None:
            print(json.dumps(inspect_export(content, tenant=args.tenant, account_name=args.account_name), indent=2))
            return 0
        if not args.account_id or not args.currency:
            raise ValueError("Conversion requires --account-id and --currency.")
        rows = convert(content, json.loads(args.mapping.read_text(encoding="utf-8")), tenant=args.tenant,
                       account_name=args.account_name, account_id=args.account_id, currency=args.currency)
        import sys
        writer = csv.DictWriter(sys.stdout, OUTPUT, lineterminator="\n")
        # Reject potentially executable spreadsheet text, without changing identifiers.
        for row in rows:
            for key in ("Tenant", "AccountID", "Reference", "Description"):
                if row[key].lstrip().startswith(("=", "+", "-", "@")):
                    raise ValueError(f"{key} starts with a spreadsheet formula prefix; review the source.")
        writer.writeheader()
        writer.writerows(rows)
        return 0
    except (OSError, UnicodeError, ValueError, csv.Error) as exc:
        import sys
        print(f"Account Transactions: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
