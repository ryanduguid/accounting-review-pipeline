from __future__ import annotations

import csv
import io
import json
import os
import re
from pathlib import Path

from .engine import CloseReviewPack
from .models import ExceptionItem


def _money(value) -> str:
    """Render a monetary Decimal with at least two decimal places, never fewer than it has.

    A fixed two-place render reports a 0.0040 difference against a 0.001
    tolerance as "0.00 exceeds 0.00" and leaves the reviewer no way back to the
    real figures, which defeats the exact-decimal arithmetic behind them.
    """
    if value is None:
        return ""
    exponent = value.as_tuple().exponent
    places = max(2, -exponent) if isinstance(exponent, int) else 2
    return f"{value:.{places}f}"


def _percentage(value) -> str:
    return "" if value is None else f"{value * 100:.2f}%"


def _exception_dict(item: ExceptionItem) -> dict[str, str]:
    return {
        "control": item.control,
        "status": item.status,
        "tenant": item.tenant,
        "account_id": item.account_id,
        "account_code": item.account_code,
        "account_name": item.account_name,
        "current_value": _money(item.current_value),
        "prior_value": _money(item.prior_value),
        "difference": _money(item.difference),
        "threshold": _money(item.threshold),
        "percentage_change": _percentage(item.percentage_change),
        "reason": item.reason,
        "reviewer_action": item.reviewer_action,
    }


_INERT_REMAINDER = re.compile(r"[\w.]*")


def _csv_safe(value: str) -> str:
    """Keep source-controlled text inert when an exceptions CSV is opened in a spreadsheet.

    '=' is always neutralised. '+', '-', and '@' are neutralised only when the
    rest of the value could be read as a formula; plain identifiers such as the
    account code '-1000' or the ID '@123' pass through unchanged so that Excel
    and Power BI joins keep working.
    """
    stripped = value.lstrip()
    if stripped.startswith("="):
        return "'" + value
    if stripped.startswith(("+", "-", "@")) and not _INERT_REMAINDER.fullmatch(stripped[1:]):
        return "'" + value
    return value


def _as_json(pack: CloseReviewPack) -> dict:
    acknowledgement = None
    if pack.acknowledgement is not None:
        acknowledgement = {
            "reviewer_initials": pack.acknowledgement.reviewer_initials,
            "reviewed_on": pack.acknowledgement.reviewed_on.isoformat(),
            "comment": pack.acknowledgement.comment,
            "effect": "Acknowledgement is evidence of human review only; it does not approve or close a period.",
        }
    return {
        "acknowledgement": acknowledgement,
        "current_report_dates": list(pack.current_report_dates),
        "exceptions": [_exception_dict(item) for item in pack.exceptions],
        "overall_status": pack.status,
        "prior_report_dates": list(pack.prior_report_dates),
        "source_sha256": dict(sorted(pack.source_hashes.items())),
        "thresholds": {
            "absolute_variance": _money(pack.absolute_threshold),
            "percentage_variance": _percentage(pack.percentage_threshold),
            "reconciliation_tolerance": _money(pack.reconciliation_tolerance),
        },
    }


def _md_cell(value: str) -> str:
    """Flatten embedded newlines and escape pipes so a value cannot break a table row."""
    return " ".join(value.split()).replace("|", "\\|")


def _as_markdown(pack: CloseReviewPack) -> str:
    blocked = sum(item.status == "BLOCKED" for item in pack.exceptions)
    review = sum(item.status == "REVIEW" for item in pack.exceptions)
    lines = [
        "# Monthly Close Review Pack",
        "",
        f"**Overall status: {pack.status}**",
        "",
        "This pack is a review aid. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.",
        "",
        "## Scope",
        "",
        f"- Current report date(s): {', '.join(pack.current_report_dates)}",
        f"- Prior report date(s): {', '.join(pack.prior_report_dates)}",
        f"- Material variance thresholds: ${_money(pack.absolute_threshold)} and {_percentage(pack.percentage_threshold)}",
        f"- Reconciliation tolerance: ${_money(pack.reconciliation_tolerance)}",
        f"- Exceptions: {len(pack.exceptions)} total; {blocked} blocked; {review} requiring review.",
        "",
        "## Source evidence",
        "",
    ]
    for name, digest in sorted(pack.source_hashes.items()):
        lines.append(f"- `{name}`: `{digest}`")
    lines += ["", "## Exceptions", ""]
    if not pack.exceptions:
        lines.append("No exceptions were raised. A human must still decide whether the close is appropriate.")
    else:
        lines += [
            "| Status | Control | Tenant | Account | Difference | Reason |",
            "| --- | --- | --- | --- | ---: | --- |",
        ]
        for item in pack.exceptions:
            account = " / ".join(piece for piece in (item.account_code, item.account_name) if piece) or item.account_id or "—"
            account = _md_cell(account)
            reason = _md_cell(item.reason)
            tenant = _md_cell(item.tenant or "—")
            lines.append(f"| {item.status} | {item.control} | {tenant} | {account} | {_money(item.difference) or '—'} | {reason} |")
    lines += ["", "## Human acknowledgement", ""]
    if pack.acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        lines += [
            f"- Reviewer initials: {_md_cell(pack.acknowledgement.reviewer_initials)}",
            f"- Reviewed on: {pack.acknowledgement.reviewed_on.isoformat()}",
            f"- Comment: {_md_cell(pack.acknowledgement.comment)}",
            "- Effect: acknowledgement records a human action only; it does not change the control status or approve a close.",
        ]
    lines.append("")
    return "\n".join(lines)


def _as_csv(pack: CloseReviewPack) -> str:
    fields = list(_exception_dict(ExceptionItem("", "PASS", "", "", "", "", None, None, None, None, None, "", "")).keys())
    buffer = io.StringIO()
    writer = csv.DictWriter(buffer, fieldnames=fields)
    writer.writeheader()
    for item in pack.exceptions:
        row = _exception_dict(item)
        for field in ("tenant", "account_id", "account_code", "account_name"):
            row[field] = _csv_safe(row[field])
        writer.writerow(row)
    return buffer.getvalue()


def _remove_quietly(path: Path) -> None:
    try:
        path.unlink()
    except OSError:
        # Cleanup is best effort; the caller re-raises the original failure.
        pass


def write_review_pack(pack: CloseReviewPack, output_dir: Path) -> dict[str, Path]:
    """Write the three pack files so a failed run cannot leave two runs mixed together.

    Each file is rendered in full, staged beside its destination, and only then
    moved into place. If a move fails - a locked exceptions.csv is the usual
    cause - the whole pack is removed rather than left as one file from this run
    beside two from the last one, because all three carry the same SHA-256
    provenance framing and a reviewer cannot tell them apart.

    exceptions.csv carries a UTF-8 byte-order mark to match the canonical input
    files, so a spreadsheet that falls back to the Windows ANSI code page does
    not turn a tenant or account name into mojibake.
    """
    output_dir.mkdir(parents=True, exist_ok=True)
    json_path = output_dir / "close-review-pack.json"
    summary_path = output_dir / "close-summary.md"
    exceptions_path = output_dir / "exceptions.csv"
    rendered = (
        (json_path, json.dumps(_as_json(pack), indent=2, sort_keys=True) + "\n", "utf-8", None),
        (summary_path, _as_markdown(pack), "utf-8", None),
        (exceptions_path, _as_csv(pack), "utf-8-sig", ""),
    )

    staged: list[tuple[Path, Path]] = []
    try:
        for destination, text, encoding, newline in rendered:
            staged_path = destination.with_name(destination.name + ".partial")
            staged_path.write_text(text, encoding=encoding, newline=newline)
            staged.append((staged_path, destination))
    except OSError:
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise

    moved: list[Path] = []
    try:
        for staged_path, destination in staged:
            os.replace(staged_path, destination)
            moved.append(destination)
    except OSError:
        for staged_path, destination in staged:
            _remove_quietly(staged_path)
            if moved:
                _remove_quietly(destination)
        raise
    return {"json": json_path, "summary": summary_path, "exceptions": exceptions_path}
