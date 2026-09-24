"""Rank the transactions behind each period_variance exception in a verified pack.

A variance exception says an account moved beyond the thresholds; the reviewer
still has to find out why. This lists the largest transactions posted to the
account between the two report dates, so the explanation starts from ledger
evidence rather than memory. It explains nothing itself: the reviewer writes
the commentary and decides whether the movement is right.
"""
from __future__ import annotations

import json
from collections import defaultdict
from datetime import date
from decimal import Context, Decimal, localcontext
from pathlib import Path
from typing import Any

from .errors import ControlInputError, DuplicateKeyError
from .loader import SourceSnapshot, _read_csv_rows, _text
from .reconciliation import COLUMNS, _date, _money
from .reconciliation_report import _csv
from .report import _csv_safe, require_output_outside_repository
from .viewer import verify_pack

BOUNDARY = (
    "Review evidence only. The listed transactions are candidates for the reviewer's "
    "explanation; they do not explain, approve or clear a variance."
)
DRIVER_COLUMNS = (
    "Tenant", "AccountID", "AccountCode", "AccountName", "Movement", "TransactionsTotal",
    "Unexplained", "Rank", "TransactionID", "Date", "Reference", "Description", "Amount",
)


def _transactions(snapshot: SourceSnapshot) -> dict[tuple[str, str], list[dict[str, Any]]]:
    path = snapshot.path
    by_account: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    seen: set[tuple[str, str, str]] = set()
    for row, values in _read_csv_rows(snapshot, COLUMNS, label="Transactions"):
        item: dict[str, Any] = {key: _text(values[key], field=key, path=path, row_number=row,
                                          allow_empty=key in {"Reference", "Description"})
                                for key in COLUMNS}
        key = (item["Tenant"], item["AccountID"], item["TransactionID"])
        if key in seen:
            raise DuplicateKeyError(f"{path}: row {row} repeats TransactionID {item['TransactionID']!r}.")
        seen.add(key)
        debit = _money(item["Debit"], field="Debit", path=path, row=row)
        credit = _money(item["Credit"], field="Credit", path=path, row=row)
        if debit < 0 or credit < 0 or (debit > 0) == (credit > 0):
            raise ControlInputError(f"{path}: row {row} needs exactly one positive Debit or Credit.")
        item["date"] = _date(item["Date"])
        item["amount"] = debit - credit
        by_account[(item["Tenant"], item["AccountID"])].append(item)
    return by_account


def variance_drivers(pack_dir: Path, transactions: Path, *, top: int = 5) -> dict[str, Any]:
    """Return the ranked drivers for every period_variance exception in the pack."""
    if top < 1:
        raise ControlInputError("--top must be at least 1.")
    document, _, _, _, artefact_sha256 = verify_pack(pack_dir)
    prior_dates, current_dates = document["prior_report_dates"], document["current_report_dates"]
    if not (isinstance(prior_dates, list) and isinstance(current_dates, list)
            and len(prior_dates) == 1 and len(current_dates) == 1):
        raise ControlInputError("The pack must carry one prior and one current report date.")
    prior, current = date.fromisoformat(prior_dates[0]), date.fromisoformat(current_dates[0])
    snapshot = SourceSnapshot.capture(transactions, label="Transactions")
    exceptions = document["exceptions"]
    assert isinstance(exceptions, list)
    accounts = []
    # Input bounds and a private context keep sums exact even if the caller changes Decimal settings.
    with localcontext(Context(prec=40)):
        by_account = _transactions(snapshot)
        for finding in exceptions:
            if finding["control"] != "period_variance":
                continue
            window = [item for item in by_account.get((finding["tenant"], finding["account_id"]), [])
                      if prior < item["date"] <= current]
            movement = Decimal(finding["difference"])
            total = sum((item["amount"] for item in window), Decimal(0))
            ranked = sorted(window, key=lambda item: (-abs(item["amount"]), item["date"],
                                                      item["TransactionID"]))[:top]
            accounts.append({
                "tenant": finding["tenant"], "account_id": finding["account_id"],
                "account_code": finding["account_code"], "account_name": finding["account_name"],
                "movement": f"{movement:.2f}", "transactions_total": f"{total:.2f}",
                "unexplained": f"{movement - total:.2f}", "transactions_in_window": len(window),
                "drivers": [{key: item[key] for key in ("TransactionID", "Date", "Reference", "Description")}
                            | {"Amount": f"{item['amount']:.2f}"} for item in ranked],
            })
    return {
        "status": "PASS" if all(Decimal(item["unexplained"]) == 0 for item in accounts) else "REVIEW",
        "boundary": BOUNDARY,
        "window": {"after": prior.isoformat(), "through": current.isoformat()},
        "top": top,
        "source_sha256": {"transactions": snapshot.sha256, **{f"pack:{name}": digest
                                                              for name, digest in artefact_sha256.items()}},
        "accounts": accounts,
    }


def write_drivers(result: dict[str, Any], output: Path) -> Path:
    """Write variance-drivers.json and variance-drivers.csv into a new directory."""
    output = require_output_outside_repository(output)
    if output.exists():
        raise ValueError("Output already exists. Choose a new directory to preserve the previous run.")
    rows = []
    for account in result["accounts"]:
        head = [_csv_safe(account[key]) for key in ("tenant", "account_id", "account_code", "account_name")]
        head += [account["movement"], account["transactions_total"], account["unexplained"]]
        # An account with no transactions in the window still gets a row, so the
        # CSV lists every variance and shows that nothing explains it.
        rows += [head + [str(rank), _csv_safe(driver["TransactionID"]), driver["Date"],
                         _csv_safe(driver["Reference"]), _csv_safe(driver["Description"]),
                         driver["Amount"]]
                 for rank, driver in enumerate(account["drivers"], start=1)] or [head + [""] * 6]
    encoded = {
        "variance-drivers.json": (json.dumps(result, indent=2, ensure_ascii=False) + "\n").encode("utf-8"),
        "variance-drivers.csv": _csv(DRIVER_COLUMNS, rows).encode("utf-8-sig"),
    }
    output.mkdir(parents=True, exist_ok=False)
    created = []
    try:
        for name, content in encoded.items():
            target = output / name
            with target.open("xb") as stream:
                created.append(target)
                stream.write(content)
    except BaseException:
        for target in reversed(created):
            target.unlink(missing_ok=True)
        output.rmdir()
        raise
    return output / "variance-drivers.csv"
