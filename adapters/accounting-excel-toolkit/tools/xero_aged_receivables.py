"""Reconcile a supplied Xero aged summary without making debtor decisions."""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import re
import sys
from datetime import date, datetime
from decimal import Decimal, localcontext
from pathlib import Path

BUCKETS = ("< 1 Month", "1 Month", "2 Months", "3 Months", "Older")
SUPPORTED_REPORT_NAME = "Aged Receivables Summary"
TOLERANCE = Decimal("0.005")
MANIFEST_FIELDS = ("report_name", "entity", "generated_at", "as_at", "ageing_basis",
                   "accounting_basis", "currency", "filters", "population", "source_format", "source_sha256")
COMPARABLE = ("entity", "as_at", "currency", "accounting_basis", "filters", "population")


def amount(text):
    """Blank exported cells are zero; missing fields and other text are errors."""
    if not isinstance(text, str):
        raise ValueError("Amounts must be supplied as decimal text.")
    text = text.strip()
    if not text:
        return Decimal(0)
    if not re.fullmatch(r"[+-]?(?:\d{1,15}|\d{1,3}(?:,\d{3}){1,4})(?:\.\d{1,4})?", text):
        raise ValueError("Expected a finite signed amount with at most 4 decimal places.")
    return Decimal(text.replace(",", ""))


def _parse_export(content):
    records = list(csv.reader(io.StringIO(content.decode("utf-8-sig"), newline=""), strict=True))
    headers = [i for i, row in enumerate(records) if row and row[0] in ("Contact", "Customer")
               and "Total" in row and all(bucket in row for bucket in BUCKETS)]
    if len(headers) != 1:
        raise ValueError("Expected exactly one aged-summary header.")
    header_at = headers[0]
    header = records[header_at]
    allowed = {header[0], "Current", *BUCKETS, "Total"}
    if len(header) != len(set(header)) or set(header) - allowed:
        raise ValueError("Duplicate, contradictory or unsupported summary headers.")
    buckets = [name for name in header if name in ("Current", *BUCKETS)]
    return records, header_at, header, buckets, hashlib.sha256(content).hexdigest()


def _confirm_cutoff(manifest, metadata, issues):
    cutoff_confirmed = False
    try:
        cutoff = date.fromisoformat(manifest.get("as_at", ""))
        if cutoff.isoformat() != manifest["as_at"]:
            raise ValueError("Noncanonical date")
        date_lines = [value[6:] for value in metadata if value.startswith("As at ")]
        cutoff_confirmed = (len(date_lines) == 1
                            and datetime.strptime(date_lines[0], "%d %B %Y").date() == cutoff)
        if not cutoff_confirmed:
            issues.append("Export and manifest cut-off dates do not agree.")
    except (ValueError, TypeError):
        issues.append("Missing or unconfirmed ISO cut-off date.")
    return cutoff_confirmed


def _review_metadata(records, header_at, manifest, digest, issues):
    for key in MANIFEST_FIELDS:
        if not isinstance(manifest.get(key), str) or not manifest[key].strip():
            issues.append(f"Missing manifest evidence: {key}.")
    if manifest.get("source_sha256") != digest:
        issues.append("Manifest does not identify the exact source bytes.")
    metadata_records = [(index, row[0].strip()) for index, row in enumerate(records[:header_at])
                        if row and row[0].strip()]
    metadata = [value for _, value in metadata_records]
    source_report_name = records[0][0].strip() if records and records[0] else ""
    if source_report_name != SUPPORTED_REPORT_NAME:
        issues.append("The first CSV record does not declare the supported report title.")
    report_name = manifest.get("report_name")
    if isinstance(report_name, str) and report_name.strip() and report_name != SUPPORTED_REPORT_NAME:
        issues.append("Manifest report_name does not declare the supported report title.")
    entities = [value for index, value in metadata_records if index != 0
                and not value.startswith(("As at ", "Ageing by "))]
    entity_confirmed = entities == [manifest.get("entity")]
    if not entity_confirmed:
        issues.append("Export metadata has a missing, duplicate or contradictory entity.")
    cutoff_confirmed = _confirm_cutoff(manifest, metadata, issues)
    try:
        generated = manifest.get("generated_at", "")
        timestamp = datetime.fromisoformat(generated)
        if "T" not in generated or timestamp.tzinfo is None:
            raise ValueError("Expected a timestamp with a timezone")
    except (ValueError, TypeError):
        issues.append("Missing or malformed generated timestamp.")
    ageing_lines = [value for value in metadata if value.startswith("Ageing by ")]
    if ageing_lines != [f"Ageing by {manifest.get('ageing_basis', '')}"]:
        issues.append("Export metadata has a missing, duplicate or contradictory ageing basis.")
    footer_record = manifest.get("footer_record")
    # Only built-in integers identify logical records; bool/subclasses are rejected.
    if footer_record is not None and (type(footer_record) is not int or footer_record <= header_at + 1):  # pylint: disable=unidiomatic-typecheck
        raise ValueError("footer_record must identify a source record after the header.")
    if footer_record is None:
        issues.append("Terminal export footer has not been identified.")
    return entity_confirmed, cutoff_confirmed, footer_record


def _reconcile_rows(records, header_at, header, buckets, footer_record, tie, issues):
    rows = []
    contact_columns = {name: Decimal(0) for name in buckets}
    footer = None
    footer_values = None
    for number, record in enumerate(records[header_at + 1:], header_at + 2):
        if not record or not any(value.strip() for value in record):
            continue
        if len(record) != len(header):
            raise ValueError(f"Source record {number}: field count does not match the header.")
        if footer is not None:
            if record[0].strip() == "Percentage of total":
                continue
            raise ValueError("Unexpected record after the designated terminal footer.")
        if not record[0].strip():
            raise ValueError(f"Source record {number}: missing contact label.")
        values = {name: amount(record[header.index(name)]) for name in (*buckets, "Total")}
        bucket_sum = sum((values[name] for name in buckets), Decimal(0))
        if number == footer_record:
            if record[0].strip() != "Total":
                raise ValueError("Designated footer does not have the Total label.")
            footer_values = values
            footer = values["Total"]
            tie("footer buckets to total", bucket_sum, footer)
            continue
        if all(not record[header.index(name)].strip() for name in (*buckets, "Total")):
            issues.append(f"Source record {number}: blank amounts cannot establish a contact row; sectioned exports need review.")
        tie(f"record {number} buckets to total", bucket_sum, values["Total"])
        for name in buckets:
            contact_columns[name] += values[name]
        rows.append({"source_record": number, "Contact": record[0],
                     **{name: str(value) for name, value in values.items()}})
    return rows, footer_values, contact_columns


def _review_footer(rows, footer_values, contact_columns, buckets, tie, issues):
    footer = None if footer_values is None else footer_values["Total"]
    if not rows:
        issues.append("No contact rows supplied; completeness is unconfirmed.")
    row_total = sum((Decimal(row["Total"]) for row in rows), Decimal(0))
    if footer is None:
        issues.append("Supplied footer record is absent; export total is unconfirmed.")
    else:
        tie("contact totals to export footer", row_total, footer)
        for name in buckets:
            tie(f"contact column {name} to export footer {name}",
                contact_columns[name], footer_values[name])
    return footer


def _review_screen_and_control(manifest, control, rows, footer, entity_confirmed, cutoff_confirmed, tie, issues):
    screen_rows = manifest.get("on_screen_rows")
    # Exact built-in row counts cannot be bool or an int subclass.
    if type(screen_rows) is not int or screen_rows < 0 or screen_rows != len(rows):  # pylint: disable=unidiomatic-typecheck
        issues.append("On-screen contact row count is missing or does not agree.")
    if "on_screen_total" not in manifest or not str(manifest.get("on_screen_total", "")).strip():
        issues.append("On-screen report total was not checked.")
    elif footer is not None:
        tie("export footer to on-screen total", footer, amount(manifest["on_screen_total"]))
    if control is None:
        issues.append("Receivables control evidence is missing.")
    else:
        comparable = (entity_confirmed and cutoff_confirmed
                      and all(isinstance(manifest.get(key), str) and manifest[key].strip()
                              and control.get(key) == manifest[key] for key in COMPARABLE))
        if not comparable or control.get("sign_convention") != "debit-positive":
            issues.append("Source or control metadata is unconfirmed or differs; control tie is unsupported.")
        if not isinstance(control.get("source"), str) or not control["source"].strip():
            issues.append("Control source reference is missing.")
            comparable = False
        if "amount" not in control or not str(control.get("amount", "")).strip():
            issues.append("Receivables control amount is missing.")
        else:
            control_amount = amount(control["amount"])
            if comparable and control.get("sign_convention") == "debit-positive" and footer is not None:
                tie("export footer to receivables control", footer, control_amount)


def inspect_export(content, manifest, control=None):
    """Tie buckets, exported footer and independently supplied control separately."""
    if not isinstance(manifest, dict) or (control is not None and not isinstance(control, dict)):
        raise ValueError("Manifest and control must be JSON objects.")
    records, header_at, header, buckets, digest = _parse_export(content)
    issues: list[str] = []
    ties: list[dict] = []
    entity_confirmed, cutoff_confirmed, footer_record = _review_metadata(
        records, header_at, manifest, digest, issues)
    with localcontext() as context:
        context.prec = 60

        def tie(label, left, right):
            delta = left - right
            agrees = abs(delta) < TOLERANCE
            ties.append({"check": label, "left": str(left), "right": str(right),
                         "difference": str(delta), "agrees": agrees})
            if not agrees:
                issues.append(f"{label}: unresolved difference {delta}.")

        rows, footer_values, contact_columns = _reconcile_rows(
            records, header_at, header, buckets, footer_record, tie, issues)
        footer = _review_footer(rows, footer_values, contact_columns, buckets, tie, issues)
        _review_screen_and_control(
            manifest, control, rows, footer, entity_confirmed, cutoff_confirmed, tie, issues)
    return {"summary_status": "REVIEW" if issues else "PASS", "debtor_decisions": "REVIEW",
            "source_sha256": digest, "manifest": manifest, "control": control,
            "rows": rows, "ties": ties, "exceptions": issues,
            "boundary": "Summary arithmetic only. Detail, subsequent receipts and authorised review are required for debtor decisions."}


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate JSON field.")
        result[key] = value
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--control", type=Path)
    args = parser.parse_args(argv)
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
        control = json.loads(args.control.read_text(encoding="utf-8"), object_pairs_hook=unique_object) if args.control else None
        result = inspect_export(args.source.read_bytes(), manifest, control)
        print(json.dumps(result, indent=2, ensure_ascii=False))
        return 0 if result["summary_status"] == "PASS" else 2
    except (OSError, UnicodeError, ValueError, csv.Error) as exc:
        print(json.dumps({"summary_status": "INVALID_INPUT", "error": str(exc)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
