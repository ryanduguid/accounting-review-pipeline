"""Read-only display of an existing close-review pack.

Phase B of the workbench: load the three generated artefacts, prove they agree
with each other before showing anything, and render a review sheet. The viewer
never writes, renames or deletes a file, never opens a network connection, and
never changes what the engine computed. A tampered, partial or mismatched
artefact set fails closed with a named error instead of being displayed.

Every check here re-reads what ``report.write_review_pack`` emitted. The two
renderers are independent witnesses of one engine run: if their contents stop
agreeing, the pack is no longer trustworthy evidence and the reviewer must hear
that from this command rather than infer it from a plausible-looking sheet.
"""

from __future__ import annotations

import hashlib
import json
import re
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .errors import ControlInputError

PACK_FILE_NAMES = ("close-review-pack.json", "close-summary.md", "exceptions.csv")

_JSON_NAME = "close-review-pack.json"
_SUMMARY_NAME = "close-summary.md"
_CSV_NAME = "exceptions.csv"

# The exact top-level members report._as_json emits, no more and no less. An
# added or removed member means the file was edited by something other than
# the writer that produced the other two artefacts.
_JSON_MEMBERS = frozenset(
    {
        "acknowledgement",
        "current_report_dates",
        "exceptions",
        "overall_status",
        "prior_report_dates",
        "source_sha256",
        "thresholds",
    }
)

_THRESHOLD_KEYS = ("absolute_variance", "percentage_variance", "reconciliation_tolerance")

# The members report._as_json writes inside an acknowledgement, all of them
# strings. The sheet prints initials, reviewed_on and comment verbatim and
# states its own effect line, so effect is shape-checked here, not displayed.
_ACKNOWLEDGEMENT_KEYS = ("reviewer_initials", "reviewed_on", "comment", "effect")

_STATUSES = ("PASS", "REVIEW", "BLOCKED")

_CSV_FIELDS = (
    "control",
    "status",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "current_value",
    "prior_value",
    "difference",
    "threshold",
    "percentage_change",
    "reason",
    "reviewer_action",
)

# Fields report._csv_safe guards with a leading apostrophe on the CSV side. It
# tests the value after lstrip, so the mirror below must strip too or a guarded
# value carrying leading whitespace looks like a tampered cell.
_CSV_GUARDED_FIELDS = frozenset(
    {"tenant", "account_id", "account_code", "account_name", "review_group"}
)

_BOUNDARY_SENTENCE = (
    "This pack is a review aid. It does not approve a close, post a journal, "
    "make a payment, lodge a return, or lock a period."
)

_STATUS_LINE = re.compile(r"\*\*Overall status: (PASS|REVIEW|BLOCKED)\*\*")

_SOURCE_EVIDENCE_LINE = re.compile(r"`([a-z_0-9]+)`: `([0-9a-f]{64})`")


def _no_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    """Reject a JSON object that states any member twice.

    A duplicated member is not valid evidence of anything: the two positions
    disagree about the pack, and standard json parsing would silently keep the
    last one, hiding the disagreement this command exists to surface.
    """
    seen: dict[str, object] = {}
    for key, value in pairs:
        if key in seen:
            raise ControlInputError(
                f"{_JSON_NAME}: member {key!r} appears more than once"
            )
        seen[key] = value
    return seen


def _load_artefact_bytes(pack_dir: Path) -> dict[str, bytes]:
    payloads: dict[str, bytes] = {}
    for name in PACK_FILE_NAMES:
        path = pack_dir / name
        try:
            payloads[name] = path.read_bytes()
        except FileNotFoundError as exc:
            raise ControlInputError(f"{name}: not found in {pack_dir}") from exc
        except IsADirectoryError as exc:
            raise ControlInputError(f"{name}: expected a file, found a directory") from exc
        except OSError as exc:
            raise ControlInputError(f"{name}: could not be read from {pack_dir} ({exc})") from exc
    return payloads


def _parse_json(payload: bytes) -> dict[str, object]:
    try:
        document = json.loads(payload.decode("utf-8"), object_pairs_hook=_no_duplicate_keys)
    except UnicodeDecodeError as exc:
        raise ControlInputError(f"{_JSON_NAME}: not valid UTF-8") from exc
    except json.JSONDecodeError as exc:
        raise ControlInputError(f"{_JSON_NAME}: not valid JSON ({exc.msg})") from exc
    if not isinstance(document, dict):
        raise ControlInputError(f"{_JSON_NAME}: top level must be a JSON object")
    members = set(document)
    unknown = sorted(members - _JSON_MEMBERS)
    if unknown:
        raise ControlInputError(
            f"{_JSON_NAME}: unknown top-level member(s): {', '.join(unknown)}"
        )
    missing = sorted(_JSON_MEMBERS - members)
    if missing:
        raise ControlInputError(
            f"{_JSON_NAME}: missing top-level member(s): {', '.join(missing)}"
        )
    return document


def _require_string_list(document: dict[str, object], member: str) -> list[str]:
    values = document[member]
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise ControlInputError(f"{_JSON_NAME}: {member} must be a list of strings")
    return values


def _parse_threshold(text: object, key: str) -> Decimal:
    """Parse a threshold rendered by report._money or report._percentage."""
    if not isinstance(text, str) or not text.endswith("%"):
        candidate = text if isinstance(text, str) else None
    else:
        candidate = text[:-1]
    if candidate is None:
        raise ControlInputError(f"{_JSON_NAME}: thresholds.{key} must be a string")
    try:
        value = Decimal(candidate)
    except InvalidOperation as exc:
        raise ControlInputError(
            f"{_JSON_NAME}: thresholds.{key} is not a decimal: {text!r}"
        ) from exc
    if not value.is_finite() or value < 0:
        raise ControlInputError(
            f"{_JSON_NAME}: thresholds.{key} must be finite and non-negative"
        )
    return value


def _verify_json_schema(document: dict[str, object]) -> None:
    status = document["overall_status"]
    if status not in _STATUSES:
        raise ControlInputError(
            f"{_JSON_NAME}: overall_status must be one of "
            f"{', '.join(_STATUSES)}; got {status!r}"
        )
    _require_string_list(document, "current_report_dates")
    _require_string_list(document, "prior_report_dates")

    thresholds = document["thresholds"]
    if not isinstance(thresholds, dict) or set(thresholds) != set(_THRESHOLD_KEYS):
        raise ControlInputError(
            f"{_JSON_NAME}: thresholds must hold exactly "
            f"{', '.join(_THRESHOLD_KEYS)}"
        )
    for key in _THRESHOLD_KEYS:
        _parse_threshold(thresholds[key], key)

    source_hashes = document["source_sha256"]
    if not isinstance(source_hashes, dict) or not source_hashes:
        raise ControlInputError(f"{_JSON_NAME}: source_sha256 must be a non-empty object")
    for label, digest in source_hashes.items():
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ControlInputError(
                f"{_JSON_NAME}: source_sha256[{label!r}] is not a lowercase SHA-256 digest"
            )

    exceptions = document["exceptions"]
    if not isinstance(exceptions, list):
        raise ControlInputError(f"{_JSON_NAME}: exceptions must be a list")
    for index, item in enumerate(exceptions):
        if not isinstance(item, dict):
            raise ControlInputError(f"{_JSON_NAME}: exceptions[{index}] must be an object")
        if item.get("status") not in _STATUSES:
            raise ControlInputError(
                f"{_JSON_NAME}: exceptions[{index}].status is not a pack status"
            )

    # An acknowledgement records a human action, so a malformed one must be
    # named here rather than reach the sheet as a traceback or as text the
    # renderer never checked.
    acknowledgement = document["acknowledgement"]
    if acknowledgement is not None:
        if not isinstance(acknowledgement, dict):
            raise ControlInputError(
                f"{_JSON_NAME}: acknowledgement must be null or an object"
            )
        for key in _ACKNOWLEDGEMENT_KEYS:
            if not isinstance(acknowledgement.get(key), str):
                raise ControlInputError(
                    f"{_JSON_NAME}: acknowledgement.{key} must be a string"
                )


def _summary_source_evidence(summary_text: str) -> dict[str, str]:
    """Collect the summary's digest lines, rejecting a contradicted label.

    As with a duplicated JSON member, keeping the last of two disagreeing
    digest lines for one source hides the disagreement: a falsified line
    paired with a duplicate carrying the true digest would then agree with
    the JSON pack and display. The whole document is scanned, so a forged
    second "## Source evidence" heading opens no unchecked region. A
    reviewer acknowledgement may legitimately quote a digest line, and an
    identical repeat states no second claim, so only a differing repeat is
    a disagreement.
    """
    found: dict[str, str] = {}
    for label, digest in _SOURCE_EVIDENCE_LINE.findall(summary_text):
        if found.get(label, digest) != digest:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: source-evidence label {label!r} appears "
                "with two different digests"
            )
        found[label] = digest
    if not found:
        raise ControlInputError(f"{_SUMMARY_NAME}: no source-evidence digest lines found")
    return found


def _verify_cross_file_agreement(
    document: dict[str, object],
    summary_text: str,
    csv_rows: list[dict[str, str]],
) -> None:
    status = document["overall_status"]
    assert isinstance(status, str)

    status_lines = _STATUS_LINE.findall(summary_text)
    if len(status_lines) != 1:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: expected exactly one overall-status line, "
            f"found {len(status_lines)}"
        )
    if status_lines[0] != status:
        raise ControlInputError(
            f"overall status disagrees: {_JSON_NAME} says {status}, "
            f"{_SUMMARY_NAME} says {status_lines[0]}"
        )

    if _BOUNDARY_SENTENCE not in summary_text:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the review-boundary statement is missing or altered"
        )

    summary_hashes = _summary_source_evidence(summary_text)
    json_hashes = document["source_sha256"]
    assert isinstance(json_hashes, dict)
    if summary_hashes != json_hashes:
        raise ControlInputError(
            f"source evidence disagrees: {_SUMMARY_NAME} and {_JSON_NAME} "
            f"list different source digests"
        )

    exceptions = document["exceptions"]
    assert isinstance(exceptions, list)
    if len(csv_rows) != len(exceptions):
        raise ControlInputError(
            f"exception counts disagree: {_JSON_NAME} holds {len(exceptions)}, "
            f"{_CSV_NAME} holds {len(csv_rows)} data rows"
        )
    for index, (item, row) in enumerate(zip(exceptions, csv_rows)):
        for field in _CSV_FIELDS:
            expected = item.get(field)
            if not isinstance(expected, str):
                raise ControlInputError(
                    f"{_JSON_NAME}: exceptions[{index}].{field} must be a string"
                )
            actual = row.get(field)
            if actual is None:
                raise ControlInputError(f"{_CSV_NAME}: row {index + 1} has no {field} column value")
            if field in _CSV_GUARDED_FIELDS and expected.lstrip().startswith(("=", "+", "-", "@")):
                expected = "'" + expected
            if actual != expected:
                raise ControlInputError(
                    f"{field} disagrees on exception {index + 1}: {_JSON_NAME} says "
                    f"{expected!r}, {_CSV_NAME} says {actual!r}"
                )


def _read_csv_rows(payload: bytes) -> list[dict[str, str]]:
    import csv
    import io

    try:
        text = payload.decode("utf-8-sig")
    except UnicodeDecodeError as exc:
        raise ControlInputError(f"{_CSV_NAME}: not valid UTF-8") from exc
    reader = csv.DictReader(io.StringIO(text, newline=""))
    if reader.fieldnames != list(_CSV_FIELDS):
        raise ControlInputError(
            f"{_CSV_NAME}: header row does not match the written contract"
        )
    return [
        {key: ("" if value is None else value) for key, value in row.items()}
        for row in reader
    ]


def verify_pack(pack_dir: Path) -> tuple[
    dict[str, object],
    str,
    list[dict[str, str]],
    dict[str, str],
]:
    """Verify one artefact set end to end and return its parsed contents.

    The returned mapping also carries the SHA-256 of each artefact's exact
    bytes under ``artefact_sha256``, so a displayed sheet can state what it
    actually read.
    """
    payloads = _load_artefact_bytes(pack_dir)
    document = _parse_json(payloads[_JSON_NAME])
    _verify_json_schema(document)
    try:
        summary_text = payloads[_SUMMARY_NAME].decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ControlInputError(f"{_SUMMARY_NAME}: not valid UTF-8") from exc
    csv_rows = _read_csv_rows(payloads[_CSV_NAME])
    _verify_cross_file_agreement(document, summary_text, csv_rows)
    artefact_digests = {
        name: hashlib.sha256(payload).hexdigest() for name, payload in payloads.items()
    }
    return document, summary_text, csv_rows, artefact_digests


def render_review_sheet(pack_dir: Path) -> tuple[str, dict[str, str]]:
    """Verify a pack and render it as a plain-text review sheet.

    Returns the sheet and the per-artefact digests it displays. Raises
    ``ControlInputError`` instead of rendering whenever any artefact is
    missing, malformed or inconsistent with its siblings.
    """
    document, summary_text, csv_rows, artefact_digests = verify_pack(pack_dir)
    thresholds = document["thresholds"]
    assert isinstance(thresholds, dict)
    exceptions = document["exceptions"]
    assert isinstance(exceptions, list)
    acknowledgement = document["acknowledgement"]

    blocked = sum(1 for item in exceptions if item["status"] == "BLOCKED")
    review = sum(1 for item in exceptions if item["status"] == "REVIEW")

    lines = [
        "Close Review Sheet",
        "",
        f"Overall status: {document['overall_status']}",
        "",
        "Review states only. This sheet does not approve a close, post a journal, make a payment, lodge a return, or lock a period.",
        "",
        "Scope",
        "",
    ]
    current_dates = document["current_report_dates"]
    assert isinstance(current_dates, list)
    prior_dates = document["prior_report_dates"]
    assert isinstance(prior_dates, list)
    lines.append(f"- Current report date(s): {', '.join(current_dates) or 'n/a'}")
    lines.append(f"- Prior report date(s): {', '.join(prior_dates) or 'n/a'}")
    lines.append(f"- Material variance thresholds: {thresholds['absolute_variance']} and {thresholds['percentage_variance']}")
    lines.append(f"- Reconciliation tolerance: {thresholds['reconciliation_tolerance']}")
    lines.append(
        f"- Exceptions: {len(exceptions)} total; {blocked} blocked; {review} requiring review."
    )
    lines += ["", "Source evidence", ""]
    source_hashes = document["source_sha256"]
    assert isinstance(source_hashes, dict)
    for label, digest in sorted(source_hashes.items()):
        lines.append(f"- {label}: {digest}")
    lines += ["", "Exceptions", ""]
    if not exceptions:
        lines.append("No exceptions were raised. A human must still decide whether the close is appropriate.")
    else:
        width = max(len(str(index + 1)) for index in range(len(exceptions)))
        for index, item in enumerate(exceptions):
            account = " / ".join(
                piece
                for piece in (item.get("account_code"), item.get("account_name"))
                if piece
            ) or item.get("account_id") or "n/a"
            lines.append(
                f"[{str(index + 1).rjust(width)}] {item['status']} {item['control']}"
                f" | {item['tenant'] or 'n/a'} | {account}"
                f" | difference {item['difference'] or 'n/a'}"
            )
            lines.append(f"     reason: {item['reason']}")
            lines.append(f"     action: {item['reviewer_action']}")
    lines += ["", "Human acknowledgement", ""]
    if acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        assert isinstance(acknowledgement, dict)
        lines.append(f"- Reviewer initials: {acknowledgement['reviewer_initials']}")
        lines.append(f"- Reviewed on: {acknowledgement['reviewed_on']}")
        comment = acknowledgement["comment"]
        if comment:
            lines.append(f"- Comment: {comment}")
        lines.append(
            "- Effect: acknowledgement records a human action only; it does not change the control status or approve a close."
        )
    lines += ["", "Artefacts verified", ""]
    for name in PACK_FILE_NAMES:
        lines.append(f"- {name}: sha256 {artefact_digests[name]}")
    lines.append("")
    return "\n".join(lines), artefact_digests
