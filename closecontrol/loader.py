from __future__ import annotations

import csv
import hashlib
import io
import json
import re
import unicodedata
from dataclasses import dataclass
from datetime import date
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .errors import ControlInputError
from .models import ReviewerAcknowledgement, TrialBalanceRow


CANONICAL_COLUMNS = (
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
)
MAPPING_COLUMNS = ("AccountID", "ReviewGroup")
SUBLEDGER_COLUMNS = ("Tenant", "AccountID", "SubledgerBalance")
_ACCOUNTING_NUMBER = re.compile(r"^[-+]?\$?(?:\d{1,3}(?:,\d{3})*|\d+)(?:\.\d+)?$")


@dataclass(frozen=True, slots=True)
class SourceSnapshot:
    """One immutable read of a source file and the digest of those exact bytes."""

    path: Path
    content: bytes
    sha256: str

    @classmethod
    def capture(cls, path: Path, *, label: str) -> SourceSnapshot:
        try:
            content = path.read_bytes()
        except FileNotFoundError as exc:
            raise ControlInputError(f"{label} does not exist: {path}.") from exc
        except OSError as exc:
            raise ControlInputError(f"{label} could not be read: {path} ({exc}).") from exc
        return cls(path=path, content=content, sha256=hashlib.sha256(content).hexdigest())

    def text(self, *, label: str, encoding: str) -> str:
        try:
            return self.content.decode(encoding)
        except UnicodeDecodeError as exc:
            raise ControlInputError(f"{label} could not be read as UTF-8: {self.path}.") from exc


def _snapshot(path: Path | SourceSnapshot, *, label: str) -> SourceSnapshot:
    if isinstance(path, SourceSnapshot):
        return path
    return SourceSnapshot.capture(path, label=label)


def _require_columns(fieldnames: list[str] | None, required: tuple[str, ...], path: Path) -> None:
    if fieldnames is None:
        raise ControlInputError(f"{path}: CSV has no header row.")
    duplicate_headers = sorted({name for name in fieldnames if fieldnames.count(name) > 1})
    if duplicate_headers:
        raise ControlInputError(f"{path}: duplicate column heading(s): {', '.join(duplicate_headers)}.")
    actual = set(fieldnames)
    expected = set(required)
    missing = sorted(expected - actual)
    unexpected = sorted(actual - expected)
    if missing or unexpected:
        detail = []
        if missing:
            detail.append(f"missing {', '.join(missing)}")
        if unexpected:
            detail.append(f"unexpected {', '.join(unexpected)}")
        raise ControlInputError(f"{path}: canonical schema mismatch ({'; '.join(detail)}).")


def _has_control_or_format_character(text: str, *, allow_line_breaks: bool = False) -> bool:
    permitted = {"\t", "\n", "\r"} if allow_line_breaks else set()
    return any(
        # Cs catches a lone surrogate. json.loads produces one from a paired
        # backslash-u escape in the D800-DFFF range, and no UTF-8 encoder
        # downstream will accept it.
        character not in permitted
        and unicodedata.category(character) in {"Cc", "Cf", "Cs"}
        for character in text
    )


def _text(
    value: str | None,
    *,
    field: str,
    row_number: int,
    path: Path,
    allow_empty: bool = False,
) -> str:
    text = (value or "").strip()
    if not text and not allow_empty:
        raise ControlInputError(f"{path}: row {row_number} has an empty {field}.")
    if _has_control_or_format_character(text):
        raise ControlInputError(
            f"{path}: row {row_number} {field} contains a control or formatting character."
        )
    return text


def parse_money(value: str | None, *, field: str, row_number: int, path: Path) -> Decimal:
    raw = (value or "").strip()
    if not raw or not _ACCOUNTING_NUMBER.fullmatch(raw):
        raise ControlInputError(f"{path}: row {row_number} has invalid {field}: {raw!r}.")
    normalised = raw.replace("$", "").replace(",", "")
    try:
        result = Decimal(normalised)
    except InvalidOperation as exc:  # Defensive: the regex should already reject malformed input.
        raise ControlInputError(f"{path}: row {row_number} has invalid {field}: {raw!r}.") from exc
    if not result.is_finite():
        raise ControlInputError(f"{path}: row {row_number} has non-finite {field}: {raw!r}.")
    return result


def load_canonical_tb(path: Path | SourceSnapshot) -> list[TrialBalanceRow]:
    snapshot = _snapshot(path, label="Trial-balance file")
    path = snapshot.path
    rows: list[TrialBalanceRow] = []
    seen: set[tuple[str, str]] = set()
    with io.StringIO(
        snapshot.text(label="Trial-balance file", encoding="utf-8-sig"), newline=""
    ) as source:
        reader = csv.DictReader(source)
        _require_columns(reader.fieldnames, CANONICAL_COLUMNS, path)
        for values in reader:
            # line_num, not an enumerate counter. DictReader silently
            # skips blank rows, so a counter drifts below the real file
            # line from the first blank line onwards and every message
            # after it names the wrong row. start=2 shows the intent was
            # always the physical line, with the header as line 1.
            row_number = reader.line_num
            if None in values:
                raise ControlInputError(f"{path}: row {row_number} has more fields than its header.")
            raw_date = _text(values["ReportDate"], field="ReportDate", row_number=row_number, path=path)
            try:
                report_date = date.fromisoformat(raw_date)
            except ValueError as exc:
                raise ControlInputError(f"{path}: row {row_number} has an invalid ISO ReportDate.") from exc
            row = TrialBalanceRow(
                report_date=report_date,
                tenant=_text(values["Tenant"], field="Tenant", row_number=row_number, path=path),
                section=_text(values["Section"], field="Section", row_number=row_number, path=path),
                account_id=_text(values["AccountID"], field="AccountID", row_number=row_number, path=path),
                account_name=_text(values["AccountName"], field="AccountName", row_number=row_number, path=path),
                account_code=_text(
                    values["AccountCode"],
                    field="AccountCode",
                    row_number=row_number,
                    path=path,
                    allow_empty=True,
                ),
                debit=parse_money(values["Debit"], field="Debit", row_number=row_number, path=path),
                credit=parse_money(values["Credit"], field="Credit", row_number=row_number, path=path),
                ytd_debit=parse_money(values["YTDDebit"], field="YTDDebit", row_number=row_number, path=path),
                ytd_credit=parse_money(values["YTDCredit"], field="YTDCredit", row_number=row_number, path=path),
            )
            if row.key in seen:
                raise ControlInputError(f"{path}: duplicate control key Tenant={row.tenant!r}, AccountID={row.account_id!r}.")
            seen.add(row.key)
            rows.append(row)
    if not rows:
        raise ControlInputError(f"{path}: no trial-balance rows were supplied.")
    tenants = {row.tenant for row in rows}
    report_dates = {row.report_date for row in rows}
    if len(tenants) != 1:
        raise ControlInputError(f"{path}: a canonical trial balance must contain exactly one tenant.")
    if len(report_dates) != 1:
        raise ControlInputError(
            f"{path}: a canonical trial balance must contain exactly one ReportDate."
        )
    return rows


def load_mapping(path: Path | SourceSnapshot | None) -> dict[str, str]:
    if path is None:
        return {}
    snapshot = _snapshot(path, label="Mapping file")
    path = snapshot.path
    mapping: dict[str, str] = {}
    with io.StringIO(
        snapshot.text(label="Mapping file", encoding="utf-8-sig"), newline=""
    ) as source:
        reader = csv.DictReader(source)
        _require_columns(reader.fieldnames, MAPPING_COLUMNS, path)
        for values in reader:
            # line_num, not an enumerate counter. DictReader silently
            # skips blank rows, so a counter drifts below the real file
            # line from the first blank line onwards and every message
            # after it names the wrong row. start=2 shows the intent was
            # always the physical line, with the header as line 1.
            row_number = reader.line_num
            if None in values:
                raise ControlInputError(f"{path}: row {row_number} has more fields than its header.")
            account_id = _text(values["AccountID"], field="AccountID", row_number=row_number, path=path)
            review_group = _text(values["ReviewGroup"], field="ReviewGroup", row_number=row_number, path=path)
            if account_id in mapping:
                raise ControlInputError(f"{path}: duplicate AccountID {account_id!r}.")
            mapping[account_id] = review_group
    return mapping


def load_subledger(path: Path | SourceSnapshot | None) -> dict[tuple[str, str], Decimal]:
    if path is None:
        return {}
    snapshot = _snapshot(path, label="Subledger file")
    path = snapshot.path
    rows: dict[tuple[str, str], Decimal] = {}
    with io.StringIO(
        snapshot.text(label="Subledger file", encoding="utf-8-sig"), newline=""
    ) as source:
        reader = csv.DictReader(source)
        _require_columns(reader.fieldnames, SUBLEDGER_COLUMNS, path)
        for values in reader:
            # line_num, not an enumerate counter. DictReader silently
            # skips blank rows, so a counter drifts below the real file
            # line from the first blank line onwards and every message
            # after it names the wrong row. start=2 shows the intent was
            # always the physical line, with the header as line 1.
            row_number = reader.line_num
            if None in values:
                raise ControlInputError(f"{path}: row {row_number} has more fields than its header.")
            tenant = _text(values["Tenant"], field="Tenant", row_number=row_number, path=path)
            account_id = _text(values["AccountID"], field="AccountID", row_number=row_number, path=path)
            key = (tenant, account_id)
            if key in rows:
                raise ControlInputError(f"{path}: duplicate subledger key Tenant={tenant!r}, AccountID={account_id!r}.")
            rows[key] = parse_money(values["SubledgerBalance"], field="SubledgerBalance", row_number=row_number, path=path)
    return rows


def load_reviewer_acknowledgement(
    path: Path | SourceSnapshot | None,
) -> ReviewerAcknowledgement | None:
    if path is None:
        return None
    snapshot = _snapshot(path, label="Review-note file")
    path = snapshot.path
    try:
        payload = json.loads(snapshot.text(label="Review-note file", encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ControlInputError(f"{path}: review note is not valid JSON.") from exc
    if not isinstance(payload, dict) or set(payload) != {"reviewer_initials", "reviewed_on", "comment"}:
        raise ControlInputError(f"{path}: review note must contain exactly reviewer_initials, reviewed_on, and comment.")
    initials = payload["reviewer_initials"]
    comment = payload["comment"]
    reviewed_on = payload["reviewed_on"]
    if not isinstance(initials, str) or not initials.strip() or len(initials.strip()) > 12:
        raise ControlInputError(f"{path}: reviewer_initials must be a non-empty string of at most 12 characters.")
    if not isinstance(comment, str) or not comment.strip():
        raise ControlInputError(f"{path}: comment must be a non-empty string.")
    # The review note is untrusted input that ends up in close-summary.md as
    # evidence. A Cf character such as U+202E can reorder how a reviewer's own
    # words render, so reject the same character classes the CSV loaders do.
    # A comment is free text, so its line breaks and tabs stay legal — the
    # markdown writer already flattens and escapes them.
    if _has_control_or_format_character(initials):
        raise ControlInputError(
            f"{path}: reviewer_initials contains a control or formatting character."
        )
    if _has_control_or_format_character(comment, allow_line_breaks=True):
        raise ControlInputError(
            f"{path}: comment contains a control or formatting character."
        )
    if not isinstance(reviewed_on, str):
        raise ControlInputError(f"{path}: reviewed_on must be an ISO date string.")
    try:
        reviewed_date = date.fromisoformat(reviewed_on)
    except ValueError as exc:
        raise ControlInputError(f"{path}: reviewed_on must be an ISO date.") from exc
    return ReviewerAcknowledgement(initials.strip(), reviewed_date, comment.strip())
