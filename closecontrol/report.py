from __future__ import annotations

import csv
import io
import json
import os
import re
import uuid
from pathlib import Path

from .engine import CloseReviewPack
from .models import ExceptionItem


def _money(value) -> str:
    """Render a monetary amount with at least two decimal places, never fewer than it has.

    A fixed two-place render reports a 0.0040 difference against a 0.001
    tolerance as "0.00 exceeds 0.00" and leaves the reviewer no way back to the
    real figures, which defeats the exact-decimal arithmetic behind them.
    A Decimal drives the scale from its own exponent; an int or a float, which
    a library caller may still put in an ExceptionItem, renders at two places.
    """
    if value is None:
        return ""
    as_tuple = getattr(value, "as_tuple", None)
    places = 2
    if as_tuple is not None:
        exponent = as_tuple().exponent
        if isinstance(exponent, int):
            places = max(2, -exponent)
    return f"{value:.{places}f}"


def _percentage(value) -> str:
    """Render a ratio as a percentage at two places, or enough to show its leading digit.

    A percentage threshold finer than a hundredth of a per cent otherwise reads
    as "0.00%" in all three files, the same loss of the configured figure that
    _money exists to avoid. The scale follows the leading significant digit
    rather than the exponent, because percentage_change is a division result
    carrying the full decimal context precision and would render 28 places.
    """
    if value is None:
        return ""
    scaled = value * 100
    adjusted = getattr(scaled, "adjusted", None)
    places = max(2, -adjusted()) if adjusted is not None else 2
    return f"{scaled:.{places}f}%"


def _exception_dict(item: ExceptionItem) -> dict[str, str]:
    return {
        "control": item.control,
        "status": item.status,
        "tenant": item.tenant,
        "account_id": item.account_id,
        "account_code": item.account_code,
        "account_name": item.account_name,
        "review_group": item.review_group,
        "current_value": _money(item.current_value),
        "prior_value": _money(item.prior_value),
        "difference": _money(item.difference),
        "threshold": _money(item.threshold),
        "percentage_change": _percentage_cell(item),
        "reason": item.reason,
        "reviewer_action": item.reviewer_action,
    }


_INERT_REMAINDER = re.compile(r"[\w.]*")
# ASCII letters then digits is A1 notation, which a sheet resolves to a cell
# rather than reading as text, so "A1" is not the plain identifier the word
# character test alone would call it. The bounds are Excel's own column and row
# limits; a wider shape names no cell and stays with the identifier test.
_CELL_REFERENCE = re.compile(r"[A-Za-z]{1,3}[0-9]{1,7}")


def _plain_identifier(remainder: str) -> bool:
    return bool(_INERT_REMAINDER.fullmatch(remainder)) and not _CELL_REFERENCE.fullmatch(remainder)


def _csv_safe(value: str) -> str:
    """Keep source-controlled text inert when an exceptions CSV is opened in a spreadsheet.

    '=' is always neutralised. '+', '-' and '@' are neutralised unless what
    follows them is a plain identifier - word characters and dots, and not an
    A1-style cell reference. The account code '-1000' and the ID '@123' pass
    through unchanged so that Excel and Power BI joins keep working; '-A1',
    which a sheet resolves to whatever cell A1 holds, does not.

    That pass-through is narrow by construction rather than a test for
    everything a spreadsheet can evaluate, and this docstring should not be
    read as claiming otherwise. A bare word remainder such as '+unsafe' is
    still let through: a sheet reads it as a defined name instead of as text,
    which costs display fidelity in that one cell and calls nothing. Every
    payload that can reach outside the sheet - DDE, WEBSERVICE, HYPERLINK, a
    pipe, a bracket - carries a character the identifier test rejects.
    """
    stripped = value.lstrip()
    if stripped.startswith("="):
        return "'" + value
    if stripped.startswith(("+", "-", "@")) and not _plain_identifier(stripped[1:]):
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


# A missing tenant, account or difference is shown as text rather than as an em
# dash: runtime output stays ASCII so a pack still reads on a console or in a
# scheduler log whose code page has no dash to render.
_ABSENT = "n/a"

# A period_variance exception whose prior YTD balance was nil has no percentage
# to render: the engine leaves percentage_change as None and its reason names
# the absolute gate as the only one tested. A blank cell in that position reads
# as "no change", so the pack states the condition instead. The sentinel keeps
# the existing ASCII "n/a" convention and cannot parse as a number, so a
# spreadsheet or JSON consumer cannot mistake it for a zero percentage.
_PRIOR_ZERO_PERCENTAGE = "n/a (prior period zero)"


def _percentage_cell(item: ExceptionItem) -> str:
    """Render an exception's percentage_change, naming the prior-zero case.

    Only a period_variance exception with a nil prior value gets the sentinel:
    every other control (integrity, mapping, metadata, reconciliation) carries
    percentage_change=None because a percentage is not part of that control at
    all, and those cells stay empty as before.
    """
    if (
        item.percentage_change is None
        and item.control == "period_variance"
        and item.prior_value is not None
        and item.prior_value == 0
    ):
        return _PRIOR_ZERO_PERCENTAGE
    return _percentage(item.percentage_change)


def _md_cell(value: str) -> str:
    r"""Flatten embedded newlines and escape a value so it cannot break a table row.

    Backslashes are escaped before pipes. A Markdown parser reads a backslash as
    consuming the character after it, so escaping the pipe alone turns source
    text of `\|` into `\\|` - an escaped backslash followed by a live delimiter -
    which adds a cell to the row and shifts every column after it. Doubling the
    backslash first leaves `\\\|`, which reads as one backslash and one literal
    pipe. Order matters: escaping pipes first and backslashes second would
    re-escape the backslashes this function just added.
    """
    return " ".join(value.split()).replace("\\", "\\\\").replace("|", "\\|")


def _md_note_lines(comment: str) -> list[str]:
    r"""Render a reviewer comment without collapsing its line structure.

    A one-line comment stays inline on the list item. A multi-line comment is
    rendered as an indented blockquote under the item, one quoted line per
    source line, so a reviewer's paragraph breaks survive into the pack.
    Each line still gets the backslash-then-pipe escaping of _md_cell, and a
    line that starts with '#' is escaped so quoted text cannot forge a
    document heading. The quote marker keeps every continuation line inside
    the acknowledgement item rather than loose in the document.
    """
    if "\n" not in comment and "\r" not in comment:
        return [f"- Comment: {_md_cell(comment)}"]
    lines = ["- Comment:"]
    for raw in comment.splitlines():
        line = _md_cell(raw)
        if line.startswith("#"):
            line = "\\" + line
        lines.append(f"  > {line}".rstrip())
    return lines


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
            account = " / ".join(piece for piece in (item.account_code, item.account_name) if piece) or item.account_id or _ABSENT
            account = _md_cell(account)
            reason = _md_cell(item.reason)
            tenant = _md_cell(item.tenant or _ABSENT)
            lines.append(f"| {item.status} | {item.control} | {tenant} | {account} | {_money(item.difference) or _ABSENT} | {reason} |")
    lines += ["", "## Human acknowledgement", ""]
    if pack.acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        lines += [
            f"- Reviewer initials: {_md_cell(pack.acknowledgement.reviewer_initials)}",
            f"- Reviewed on: {pack.acknowledgement.reviewed_on.isoformat()}",
            *_md_note_lines(pack.acknowledgement.comment),
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
        for field in ("tenant", "account_id", "account_code", "account_name", "review_group"):
            row[field] = _csv_safe(row[field])
        writer.writerow(row)
    return buffer.getvalue()


def _remove_quietly(path: Path) -> None:
    try:
        path.unlink()
    except OSError:
        # Cleanup is best effort; the caller re-raises the original failure.
        pass


def _restore_quietly(parked: Path, destination: Path) -> None:
    """Put a previous run's file back where it was. Best effort, as above."""
    try:
        os.replace(parked, destination)
    except OSError:
        pass


def _sibling_partial(destination: Path) -> Path:
    """Create an empty, uniquely named file beside `destination` to hold pack content.

    A fixed `<name>.partial` lets two runs sharing an --output directory
    overwrite each other's staged file and then move the wrong pack into place.
    The file is created by an ordinary exclusive open rather than by
    tempfile.mkstemp, because a staged file becomes the pack file and mkstemp's
    owner-only mode would quietly change who can read a delivered pack.
    """
    while True:
        candidate = destination.with_name(f"{destination.name}.{uuid.uuid4().hex[:12]}.partial")
        try:
            with candidate.open("x", encoding="utf-8"):
                pass
        except FileExistsError:  # pragma: no cover - a 48-bit name collision.
            continue
        return candidate


def _swap_into_place(staged_path: Path, destination: Path) -> Path | None:
    """Move a staged file onto its destination, parking any previous content aside.

    Returns the file now holding the previous content so the caller can restore
    it if a later move fails, or None if the destination held no file. Nothing
    is destroyed here: an existing file is renamed, never unlinked.

    Only a regular file is parked. Anything else at the destination - a
    directory, most plainly - stays put and fails the move, because Windows
    renames a directory onto a file path quite happily and moving one aside is
    well beyond writing a review pack.
    """
    parked: Path | None = None
    if destination.is_file():
        parked = _sibling_partial(destination)
        try:
            os.replace(destination, parked)
        except OSError:
            _remove_quietly(parked)
            raise
    try:
        os.replace(staged_path, destination)
    except OSError:
        if parked is not None:
            _restore_quietly(parked, destination)
        raise
    return parked


PACK_FILE_NAMES = ("close-review-pack.json", "close-summary.md", "exceptions.csv")
"""The three names write_review_pack claims in its output directory."""


def write_review_pack(pack: CloseReviewPack, output_dir: Path) -> dict[str, Path]:
    """Write the three pack files so a failed run cannot leave two runs mixed together.

    Each file is rendered in full, staged beside its destination under a unique
    name, and only then moved into place. If a move fails - a locked
    exceptions.csv is the usual cause - the files this run had already moved are
    rolled back to the content they replaced, so the directory holds the whole
    previous pack rather than one file from this run beside two from the last
    one; all three carry the same SHA-256 provenance framing and a reviewer
    cannot tell them apart. Apart from the three destinations themselves, no
    file is ever deleted; a caller that points a source path at one of
    PACK_FILE_NAMES inside output_dir destroys that source, which is why the
    CLI refuses that combination before the run starts.

    Rollback is best effort against a second failure, and a run killed outright
    can leave a `.partial` file behind, so a stray `.partial` may hold either
    this run's staged content or the previous run's. Concurrent runs sharing one
    output directory are not serialised; run one at a time per directory.

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
            staged_path = _sibling_partial(destination)
            # Recorded before the write, so a write that dies part-way through
            # is cleaned up rather than left as a truncated orphan.
            staged.append((staged_path, destination))
            staged_path.write_text(text, encoding=encoding, newline=newline)
    except BaseException:
        # Not just OSError: a render that cannot be encoded raises
        # UnicodeEncodeError, a ValueError, and used to walk out of here
        # leaving a .partial holding the whole pack - tenant, accounts and
        # balances - in a directory the caller believes the run never wrote to.
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise

    replaced: list[tuple[Path, Path | None]] = []
    try:
        for staged_path, destination in staged:
            replaced.append((destination, _swap_into_place(staged_path, destination)))
    except OSError:
        for destination, parked in reversed(replaced):
            if parked is None:
                # This run created the file; nothing preceded it.
                _remove_quietly(destination)
            else:
                _restore_quietly(parked, destination)
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise
    for _, parked in replaced:
        if parked is not None:
            _remove_quietly(parked)
    return {"json": json_path, "summary": summary_path, "exceptions": exceptions_path}
