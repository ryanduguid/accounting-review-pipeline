"""Local grant allocation and funding workpapers from explicitly reviewed inputs."""
from __future__ import annotations

import argparse
import csv
import ctypes
import errno
import hashlib
import html
import io
import json
import os
import re
import sys
import tempfile
import unicodedata
from collections import defaultdict
from datetime import date
from decimal import Decimal, localcontext
from pathlib import Path


def money(value: str) -> Decimal:
    if not isinstance(value, str) or not re.fullmatch(r"\d{1,16}(?:\.\d{1,2})?", value):
        raise ValueError(f"Expected a non-negative decimal string with at most 2 places: {value!r}")
    return Decimal(value)


def iso(value: str) -> date:
    result = date.fromisoformat(value)
    if result.isoformat() != value:
        raise ValueError("Use YYYY-MM-DD dates")
    return result


def rows(content: bytes, fields: str, key: str) -> list[dict[str, str]]:
    reader = csv.DictReader(io.StringIO(content.decode("utf-8-sig")), strict=True)
    if reader.fieldnames != fields.split():
        raise ValueError(f"Expected exact CSV headings: {fields}")
    result, seen = [], set()
    for row in reader:
        if None in row or any(value is None for value in row.values()):
            raise ValueError("Every CSV row must have the exact field count")
        if not row[key].strip() or row[key] in seen:
            raise ValueError(f"Missing or duplicate {key}: {row[key]!r}")
        for field, value in row.items():
            text_field(value, field, required=bool(value.strip()))
        seen.add(row[key])
        result.append(row)
    return result


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON field: {key}")
        result[key] = value
    return result


def text_field(value, label, *, required=True):
    if not isinstance(value, str):
        raise ValueError(f"{label} must be text")
    categories = [unicodedata.category(ch) for ch in value]
    if (any(category.startswith("C") or category in {"Zl", "Zp"} for category in categories)
            or (required and not any(category[0] in "LNPS" for category in categories))):
        raise ValueError(f"{label} must be visible text without Unicode control or format characters")
    has_base = False
    for category in categories:
        if category.startswith("M"):
            if not has_base:
                raise ValueError(f"{label} text has a combining mark without a base character")
        else:
            has_base = category[0] in "LNPS"
    return value


def markdown(value):
    return re.sub(r"([\\`*_{}\[\]()#+.!|>~-])", r"\\\1", html.escape(str(value)))


def review(source: Path) -> dict:
    names = ("agreements.json", "ledger.csv", "allocations.csv", "receipts.csv")
    content = {name: (source / name).read_bytes() for name in names}
    # Optional: interest earned, cash co-contributions and in-kind contributions.
    contribution_path = source / "contributions.csv"
    try:
        contribution_path.lstat()
    except FileNotFoundError:
        pass
    else:
        content["contributions.csv"] = contribution_path.read_bytes()
    try:
        agreements = json.loads(content["agreements.json"], object_pairs_hook=unique_fields)
    except RecursionError as exc:
        raise ValueError("agreements.json is nested too deeply to read") from exc
    fields = {"schema_version", "entity", "currency", "ledger_start", "ledger_end", "amount_basis", "agreements"}
    if not isinstance(agreements, dict) or set(agreements) != fields or type(agreements["schema_version"]) is not int or agreements["schema_version"] != 1:
        raise ValueError("Unexpected agreement schema")
    text_field(agreements["entity"], "Entity")
    if agreements["currency"] != "AUD" or agreements["amount_basis"] != "GST-inclusive":
        raise ValueError("Version 1 requires one named entity, AUD and explicitly GST-inclusive amounts")
    start, end = iso(agreements["ledger_start"]), iso(agreements["ledger_end"])
    if start > end:
        raise ValueError("Ledger period is reversed")
    ledger = rows(content["ledger.csv"], "ledger_id date description amount cash_paid evidence", "ledger_id")
    allocations = rows(content["allocations.csv"], "allocation_id ledger_id grant_id budget_line amount cash_allocated approved evidence", "allocation_id")
    receipts = rows(content["receipts.csv"], "receipt_id grant_id date amount evidence", "receipt_id")
    contributions = (rows(content["contributions.csv"], "contribution_id grant_id date kind amount evidence",
                          "contribution_id") if "contributions.csv" in content else [])
    if not ledger or not isinstance(agreements["agreements"], list) or not agreements["agreements"]:
        raise ValueError("Supply ledger evidence and at least one reviewed agreement")
    with localcontext() as context:
        context.prec = 80
        return calculate(agreements, ledger, allocations, receipts, start, end, content, contributions)


CONTRIBUTION_KINDS = ("interest", "cash_contribution", "in_kind")


def calculate(header, ledger, allocations, receipts, start, end, content, contributions=()):
    grants, source_rows = {}, {}
    findings = []
    for grant in header["agreements"]:
        fields = {"grant_id", "start", "end", "opening_cash", "opening_unspent_funding", "budget", "agreement_evidence"}
        if not isinstance(grant, dict) or set(grant) != fields or text_field(grant["grant_id"], "Grant ID") in grants:
            raise ValueError("Agreement IDs must be unique with complete fields")
        text_field(grant["agreement_evidence"], "Agreement evidence")
        if iso(grant["start"]) > iso(grant["end"]):
            raise ValueError("Agreement dates and reviewed summary evidence are required")
        money(grant["opening_cash"])
        money(grant["opening_unspent_funding"])
        if not isinstance(grant["budget"], dict) or not grant["budget"]:
            raise ValueError("Supply the reviewed budget-line mapping")
        for key, value in grant["budget"].items():
            text_field(key, "Budget line")
            money(value)
        grants[grant["grant_id"]] = grant
    for row in ledger:
        if not start <= iso(row["date"]) <= end:
            raise ValueError("Ledger rows must fall inside the declared ledger period")
        expense, paid = money(row["amount"]), money(row["cash_paid"])
        if expense <= 0 or paid > expense:
            raise ValueError("Source expenses must be positive; cash paid cannot exceed the expense")
        source_rows[row["ledger_id"]] = row
        if not row["evidence"].strip():
            findings.append({"code": "MISSING_SOURCE_EVIDENCE", "reference": row["ledger_id"]})
    allocated, cash_allocated = defaultdict(Decimal), defaultdict(Decimal)
    totals, period_totals, approved_totals, paid_totals = (defaultdict(Decimal) for _ in range(4))
    details = []
    for row in allocations:
        if row["ledger_id"] not in source_rows or row["grant_id"] not in grants:
            raise ValueError("Allocation references an unknown ledger row or grant")
        grant, item = grants[row["grant_id"]], source_rows[row["ledger_id"]]
        if row["budget_line"] not in grant["budget"] or row["approved"] not in {"yes", "no"}:
            raise ValueError("Use a reviewed budget line and approved yes/no")
        expense, paid = money(row["amount"]), money(row["cash_allocated"])
        if expense <= 0 or paid > expense:
            raise ValueError("Allocation expense must be positive and cover its cash allocation")
        allocated[row["ledger_id"]] += expense
        cash_allocated[row["ledger_id"]] += paid
        if allocated[row["ledger_id"]] > money(item["amount"]) or cash_allocated[row["ledger_id"]] > money(item["cash_paid"]):
            raise ValueError("Allocations exceed a source expense or its cash paid")
        key = (row["grant_id"], row["budget_line"])
        totals[key] += expense
        paid_totals[key] += paid
        in_period = iso(grant["start"]) <= iso(item["date"]) <= iso(grant["end"])
        if in_period:
            period_totals[key] += expense
        else:
            findings.append({"code": "OUTSIDE_AGREEMENT_PERIOD", "reference": row["allocation_id"]})
        supported = bool(item["evidence"].strip() and row["evidence"].strip())
        if not supported:
            findings.append({"code": "MISSING_ALLOCATION_EVIDENCE", "reference": row["allocation_id"]})
        if row["approved"] != "yes":
            findings.append({"code": "UNAPPROVED_ALLOCATION", "reference": row["allocation_id"]})
        if in_period and supported and row["approved"] == "yes":
            approved_totals[key] += expense
        details.append({**row, "ledger_date": item["date"], "source_evidence": item["evidence"],
                        "agreement_evidence": grant["agreement_evidence"], "within_period": "yes" if in_period else "no"})
    funding = defaultdict(Decimal)
    for row in receipts:
        if row["grant_id"] not in grants or not start <= iso(row["date"]) <= end:
            raise ValueError("Funding receipt must reference a declared grant and ledger period")
        amount = money(row["amount"])
        if amount <= 0:
            raise ValueError("Funding receipts must be positive")
        funding[row["grant_id"]] += amount
        if not row["evidence"].strip():
            findings.append({"code": "MISSING_RECEIPT_EVIDENCE", "reference": row["receipt_id"]})
    # Reported beside the grant, never added to its cash or unspent funding: whether
    # interest must be spent or returned, and how a contribution counts, is the
    # agreement's term for the reviewer to apply.
    contributed = defaultdict(Decimal)
    for row in contributions:
        if row["grant_id"] not in grants or not start <= iso(row["date"]) <= end:
            raise ValueError("Contribution must reference a declared grant and ledger period")
        if row["kind"] not in CONTRIBUTION_KINDS:
            raise ValueError(f"Contribution kind must be one of {', '.join(CONTRIBUTION_KINDS)}")
        amount = money(row["amount"])
        if amount <= 0:
            raise ValueError("Contributions must be positive")
        contributed[(row["grant_id"], row["kind"])] += amount
        if not row["evidence"].strip():
            findings.append({"code": "MISSING_CONTRIBUTION_EVIDENCE", "reference": row["contribution_id"]})
    contribution_rows = [{"grant_id": identifier, **{kind: str(contributed[(identifier, kind)])
                                                     for kind in CONTRIBUTION_KINDS}} for identifier in grants]
    tieout = []
    for key, row in source_rows.items():
        unallocated = money(row["amount"]) - allocated[key]
        unpaid_allocation = money(row["cash_paid"]) - cash_allocated[key]
        tieout.append({"ledger_id": key, "source_amount": row["amount"], "allocated": str(allocated[key]),
                       "unallocated": str(unallocated), "source_cash_paid": row["cash_paid"],
                       "cash_allocated": str(cash_allocated[key]), "cash_unallocated": str(unpaid_allocation)})
        if unallocated or unpaid_allocation:
            findings.append({"code": "UNALLOCATED_SOURCE", "reference": key})
    budget_rows, movements = [], []
    for identifier, grant in grants.items():
        spent, paid = Decimal(0), Decimal(0)
        for line, budget in grant["budget"].items():
            key = (identifier, line)
            spent += totals[key]
            paid += paid_totals[key]
            budget_rows.append({"grant_id": identifier, "budget_line": line, "budget": budget,
                                "allocated": str(totals[key]), "within_period": str(period_totals[key]),
                                "approved_evidenced_in_period": str(approved_totals[key]),
                                "cash_allocated": str(paid_totals[key])})
            if totals[key] > money(budget):
                findings.append({"code": "OVER_BUDGET", "reference": f"{identifier}/{line}"})
        remaining = money(grant["opening_unspent_funding"]) + funding[identifier] - spent
        cash = money(grant["opening_cash"]) + funding[identifier] - paid
        movements.append({"grant_id": identifier, "opening_cash": grant["opening_cash"],
                          "opening_unspent_funding": grant["opening_unspent_funding"],
                          "funding_received": str(funding[identifier]), "expenditure_allocated": str(spent),
                          "cash_paid_allocated": str(paid), "closing_cash_allocation": str(cash),
                          "unspent_funding_workpaper": str(remaining)})
        if remaining < 0 or cash < 0:
            findings.append({"code": "FUNDING_SHORTFALL", "reference": identifier})
    ledger_total = sum((money(row["amount"]) for row in ledger), Decimal(0))
    allocated_total = sum(allocated.values(), Decimal(0))
    return {"schema_version": 1, "entity": header["entity"], "currency": header["currency"],
            "ledger_start": header["ledger_start"], "ledger_end": header["ledger_end"], "amount_basis": header["amount_basis"],
            "source_sha256": {name: hashlib.sha256(value).hexdigest() for name, value in content.items()},
            "status": "REVIEW" if findings else "RECONCILED", "ledger_total": str(ledger_total),
            "allocated_total": str(allocated_total), "unallocated_total": str(ledger_total - allocated_total),
            "budget_lines": budget_rows, "funding_movements": movements, "contributions": contribution_rows,
            "allocation_evidence": details,
            "ledger_tieout": tieout, "findings": findings,
            "scope": "Workpaper arithmetic and supplied review decisions only. No grant eligibility, income recognition, liability, acquittal approval or lodgement decision."}


def safe_cell(value):
    text = str(value)
    # A plain number stays a number: prefixing a shortfall such as -1320 turned
    # it into text that spreadsheet totals over funding_movements.csv skipped. A
    # leading zero ("-001") marks an identifier a spreadsheet would rewrite, so
    # only a number written the way the workpaper writes one is left alone.
    if re.fullmatch(r"-?(?:0|[1-9]\d*)(?:\.\d+)?", text):
        return text
    return "'" + text if text.lstrip().startswith(("=", "+", "-", "@")) else text


def publish_directory(staged: Path, output: Path) -> None:
    """Atomically publish a complete directory without replacing any destination."""
    if sys.platform == "win32":
        # Windows rename refuses an existing destination, including an empty directory.
        staged.rename(output)
        return
    libc = ctypes.CDLL(None, use_errno=True)
    source, destination = os.fsencode(staged), os.fsencode(output)
    try:
        if sys.platform.startswith("linux"):
            rename = libc.renameat2
            rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
            arguments = (-100, source, -100, destination, 1)  # AT_FDCWD, RENAME_NOREPLACE
        elif sys.platform == "darwin":
            rename = libc.renamex_np
            rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
            arguments = (source, destination, 4)  # RENAME_EXCL
        else:
            raise OSError(errno.ENOTSUP, "Exclusive directory publication is unsupported")
    except AttributeError as exc:
        raise OSError(errno.ENOTSUP, "Exclusive directory publication is unavailable") from exc
    rename.restype = ctypes.c_int
    if rename(*arguments) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(output))


def write(result: dict, output: Path) -> None:
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".grant-workpaper-", dir=output.parent) as temporary:
        staged = Path(temporary) / "complete"
        staged.mkdir()
        write_files(result, staged)
        publish_directory(staged, output)


def write_files(result: dict, output: Path) -> None:
    (output / "workpaper.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    for name in ("budget_lines", "funding_movements", "contributions", "allocation_evidence", "ledger_tieout", "findings"):
        table = result[name]
        fields = list(dict.fromkeys(key for row in table for key in row)) or ["code", "reference"]
        with (output / f"{name}.csv").open("x", encoding="utf-8", newline="") as stream:
            writer = csv.DictWriter(stream, fields)
            writer.writeheader()
            writer.writerows({key: safe_cell(row.get(key, "")) for key in fields} for row in table)
    lines = ["# Grant acquittal workpaper", "", f"Entity: {markdown(result['entity'])}. Currency: AUD. Basis: GST-inclusive.",
             f"Ledger period: {result['ledger_start']} to {result['ledger_end']}. Result: {result['status']}.", "",
             f"Ledger expenditure {result['ledger_total']} = allocated {result['allocated_total']} + unallocated {result['unallocated_total']}.", "",
             "See allocation_evidence.csv for each agreement, source expense and review decision; funding_movements.csv separates cash from expenditure.",
             "contributions.csv reports interest, cash co-contributions and in-kind contributions per grant; none is added to cash or unspent funding.", "",
             "| Grant | Closing cash allocation | Unspent funding workpaper |", "| --- | ---: | ---: |"]
    for row in result["funding_movements"]:
        lines.append(f"| {markdown(row['grant_id'])} | {row['closing_cash_allocation']} | {row['unspent_funding_workpaper']} |")
    lines.extend(["", "## Items requiring review", ""])
    lines.extend(f"- {markdown(row['code'])}: {markdown(row['reference'])}" for row in result["findings"])
    if not result["findings"]:
        lines.append("No differences found in the supplied evidence.")
    lines.extend(["", result["scope"], ""])
    (output / "workpaper.md").write_text("\n".join(lines), encoding="utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        result = review(args.input)
        # Input directories cannot be overwritten; every run creates a fresh folder.
        write(result, args.output)
    except (ValueError, TypeError, KeyError, OSError, csv.Error) as exc:
        print(f"grant-workpaper: {exc}", file=sys.stderr)
        return 1
    print(f"grant-workpaper: {result['status']}; {len(result['findings'])} review items")
    return 2 if result["findings"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
