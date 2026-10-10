"""Display explicitly supplied close packs through the existing verifier."""
from __future__ import annotations

import json
from collections.abc import Sequence
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import cast

from .errors import ControlInputError
from .viewer import verify_pack

PACK_STATES = ("PASS", "REVIEW", "BLOCKED")


def _cell(value: str) -> str:
    """Keep supplied text in one table cell without terminal control characters."""
    return json.dumps(value, ensure_ascii=True)[1:-1].replace("|", r"\|")


@dataclass(frozen=True)
class _QueueEntry:
    path: Path
    dates: tuple[str, ...]
    status: str
    reviewer: str | None
    row: str
    digests: dict[str, str]


def _verified_entry(path: Path) -> _QueueEntry:
    """Retain display evidence while releasing the verified pack's full payload."""
    document, _, _, _, digests = verify_pack(path)
    dates = tuple(cast(list[str], document["current_report_dates"]))
    status = str(document["overall_status"])
    exceptions = cast(list[object], document["exceptions"])
    queries = cast(list[object] | None, document.get("client_queries"))
    controls = cast(list[str] | None, document.get("controls_not_run"))
    acknowledgement = cast(dict[str, str] | None, document["acknowledgement"])
    reviewed = (
        f"{acknowledgement['reviewer_initials']} ({acknowledgement['reviewed_on']})"
        if acknowledgement is not None else "not recorded"
    )
    values = [
        path.as_posix(), ", ".join(dates) or "not recorded", status,
        str(len(exceptions)), str(len(queries)) if queries is not None else "not recorded",
        reviewed, (", ".join(controls) or "none") if controls is not None else "not recorded",
    ]
    return _QueueEntry(
        path, dates, status,
        acknowledgement["reviewer_initials"] if acknowledgement is not None else None,
        "| " + " | ".join(_cell(value) for value in values) + " |", digests,
    )


def _validate_filters(period: str | None, status: str | None, reviewer: str | None) -> None:
    """Reject invalid filters before resolving or reading any supplied pack."""
    if period is not None:
        try:
            if date.fromisoformat(period).isoformat() != period:
                raise ValueError
        except ValueError as exc:
            raise ControlInputError("Period must be a calendar date in YYYY-MM-DD form.") from exc
    if status is not None and status not in PACK_STATES:
        raise ControlInputError("Status must be PASS, REVIEW or BLOCKED.")
    if reviewer is not None and not reviewer.strip():
        raise ControlInputError("Reviewer must not be empty.")


def render_review_queue(
    pack_dirs: Sequence[Path],
    *,
    period: str | None = None,
    status: str | None = None,
    reviewer: str | None = None,
) -> str:
    """Verify every supplied pack, then filter and display their recorded evidence."""
    if not pack_dirs:
        raise ControlInputError("Supply at least one pack directory.")
    _validate_filters(period, status, reviewer)

    try:
        paths = sorted({path.resolve() for path in pack_dirs}, key=str)
    except (OSError, RuntimeError) as exc:
        raise ControlInputError(f"Cannot resolve pack directory: {exc}") from exc
    verified = []
    for path in paths:
        try:
            entry = _verified_entry(path)
        except ControlInputError as exc:
            raise ControlInputError(f"Pack {str(path)!r}: {exc}") from exc
        verified.append(entry)

    # Verify before filtering: a filter must not hide an invalid supplied pack.
    selected = [
        entry for entry in verified
        if (period is None or period in entry.dates)
        and (status is None or entry.status == status)
        and (reviewer is None or entry.reviewer == reviewer)
    ]
    selected.sort(key=lambda entry: (entry.dates, str(entry.path)))

    lines = [
        "Close Review Queue",
        "",
        f"Verified {len(verified)} distinct pack(s); displaying {len(selected)}.",
        "Recorded states are review evidence. This queue does not approve a close "
        "or change a finding, acknowledgement or source file.",
        "Recorded reviewers are acknowledgement evidence, not preparer assignments. "
        "Owners and due dates are not recorded by the pack schema.",
        "",
    ]
    if not selected:
        return "\n".join([*lines, "No verified packs match the supplied filters."])
    lines.extend([
        "| Pack | Report dates | State | Exceptions | Draft queries | Recorded reviewer | Controls not run |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ])
    lines.extend(entry.row for entry in selected)
    lines.extend(["", "Verified artefact SHA-256", ""])
    for entry in selected:
        lines.append(_cell(entry.path.as_posix()))
        lines.extend(f"  {name}: {digest}" for name, digest in sorted(entry.digests.items()))
    return "\n".join(lines)
