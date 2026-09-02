from __future__ import annotations

import csv
import hashlib
import io
import json
import unicodedata
from collections.abc import Iterator, Sequence
from dataclasses import dataclass
from datetime import date
from decimal import Decimal, InvalidOperation
from pathlib import Path
import re

from .errors import (
    DateMismatchError,
    DuplicateKeyError,
    GateInputError,
    NumericGateError,
    SchemaError,
)
from .models import (
    ActivityLabel,
    BankRecRow,
    GstControlRow,
    OpenItem,
    PriorFinding,
    ReviewerAcknowledgement,
    SelfReview,
    TieOutRow,
    TrialBalanceRow,
)


CANONICAL_TB_COLUMNS = (
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
OPEN_ITEM_COLUMNS = (
    "ItemID",
    "Severity",
    "Owner",
    "DueDate",
    "Status",
    "Description",
    "Resolution",
)
ACTIVITY_COLUMNS = ("Label", "Amount")
GST_CONTROL_COLUMNS = ("Date", "AccountID", "AccountName", "Debit", "Credit", "Description")
BANK_REC_COLUMNS = ("AccountID", "StatementBalance", "GLBalance")
TIE_OUT_COLUMNS = ("StatementLine", "StatementAmount", "WorkpaperRef", "SourceFile", "Status")
PRIOR_FINDING_COLUMNS = ("FindingCode", "Slot", "Status")
SELF_REVIEW_KEYS = frozenset(
    {"preparer_initials", "prepared_on", "engagement_type", "period_end", "assertions"}
)
SELF_REVIEW_ASSERTIONS = (
    "pack_complete",
    "tie_outs_done",
    "open_items_listed",
    "variances_explained",
    "self_reviewed",
)
OPEN_ITEM_SEVERITIES = {"BLOCKING", "EXPLAIN", "TRIVIAL"}
OPEN_ITEM_STATUSES = {"OPEN", "CLEARED"}
TIE_OUT_STATUSES = {"TIED", "EXCEPTION", "UNSUPPORTED"}
_ACCOUNTING_NUMBER = re.compile(r"^[-+]?\$?(?:\d{1,3}(?:,\d{3})*|\d+)(?:\.\d+)?$")
_EXTENDED_ISO_DATE = re.compile(r"\d{4}-\d{2}-\d{2}")


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
            raise GateInputError(f"{label} does not exist: {path}.") from exc
        except OSError as exc:
            raise GateInputError(f"{label} could not be read: {path} ({exc}).") from exc
        return cls(path=path, content=content, sha256=hashlib.sha256(content).hexdigest())

    def text(self, *, label: str, encoding: str) -> str:
        try:
            return self.content.decode(encoding)
        except UnicodeDecodeError as exc:
            raise GateInputError(f"{label} could not be read as UTF-8: {self.path}.") from exc


def _require_columns(
    fieldnames: Sequence[str] | None, required: tuple[str, ...], path: Path
) -> None:
    if fieldnames is None:
        raise SchemaError(f"{path}: CSV has no header row.")
    duplicate_headers = sorted({name for name in fieldnames if fieldnames.count(name) > 1})
    if duplicate_headers:
        raise SchemaError(f"{path}: duplicate column heading(s): {', '.join(duplicate_headers)}.")
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
        raise SchemaError(f"{path}: canonical schema mismatch ({'; '.join(detail)}).")


def _read_csv_rows(
    snapshot: SourceSnapshot, required: tuple[str, ...], *, label: str
) -> Iterator[tuple[int, dict[str, str | None]]]:
    path = snapshot.path
    with io.StringIO(snapshot.text(label=label, encoding="utf-8-sig"), newline="") as source:
        reader = csv.DictReader(source)
        _require_columns(reader.fieldnames, required, path)
        for values in reader:
            row_number = reader.line_num
            if None in values:
                raise SchemaError(f"{path}: row {row_number} has more fields than its header.")
            yield row_number, values


def _has_control_or_format_character(text: str, *, allow_line_breaks: bool = False) -> bool:
    permitted = {"\t", "\n", "\r"} if allow_line_breaks else set()
    return any(
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
        raise SchemaError(f"{path}: row {row_number} has an empty {field}.")
    if _has_control_or_format_character(text):
        raise SchemaError(
            f"{path}: row {row_number} {field} contains a control or formatting character."
        )
    return text


def parse_money(value: str | None, *, field: str, row_number: int, path: Path) -> Decimal:
    raw = (value or "").strip()
    if not raw or not _ACCOUNTING_NUMBER.fullmatch(raw):
        raise NumericGateError(f"{path}: row {row_number} has invalid {field}: {raw!r}.")
    normalised = raw.replace("$", "").replace(",", "")
    try:
        result = Decimal(normalised)
    except InvalidOperation as exc:
        raise NumericGateError(f"{path}: row {row_number} has invalid {field}: {raw!r}.") from exc
    if not result.is_finite():
        raise NumericGateError(f"{path}: row {row_number} has non-finite {field}: {raw!r}.")
    return result


def parse_iso_date(value: str, *, field: str, path: Path, row_number: int | None = None) -> date:
    try:
        # date.fromisoformat also accepts basic-format and ISO-week strings from
        # 3.11 on. Insist on the extended calendar date schemas/self_review.json
        # declares, so the same pack gates identically on every supported version.
        if not _EXTENDED_ISO_DATE.fullmatch(value):
            raise ValueError(f"{value!r} is not an extended-format ISO date")
        return date.fromisoformat(value)
    except ValueError as exc:
        where = f" row {row_number}" if row_number is not None else ""
        raise SchemaError(f"{path}:{where} {field} must be an ISO date.".replace(": ", ":")) from exc


def load_canonical_tb(snapshot: SourceSnapshot) -> list[TrialBalanceRow]:
    path = snapshot.path
    rows: list[TrialBalanceRow] = []
    seen: set[tuple[str, str]] = set()
    for row_number, values in _read_csv_rows(
        snapshot, CANONICAL_TB_COLUMNS, label="Trial-balance file"
    ):
        report_date = parse_iso_date(
            _text(values["ReportDate"], field="ReportDate", row_number=row_number, path=path),
            field="ReportDate",
            path=path,
            row_number=row_number,
        )
        row = TrialBalanceRow(
            report_date=report_date,
            tenant=_text(values["Tenant"], field="Tenant", row_number=row_number, path=path),
            section=_text(values["Section"], field="Section", row_number=row_number, path=path),
            account_id=_text(values["AccountID"], field="AccountID", row_number=row_number, path=path),
            account_name=_text(
                values["AccountName"], field="AccountName", row_number=row_number, path=path
            ),
            account_code=_text(
                values["AccountCode"],
                field="AccountCode",
                row_number=row_number,
                path=path,
                allow_empty=True,
            ),
            debit=parse_money(values["Debit"], field="Debit", row_number=row_number, path=path),
            credit=parse_money(values["Credit"], field="Credit", row_number=row_number, path=path),
            ytd_debit=parse_money(
                values["YTDDebit"], field="YTDDebit", row_number=row_number, path=path
            ),
            ytd_credit=parse_money(
                values["YTDCredit"], field="YTDCredit", row_number=row_number, path=path
            ),
        )
        if row.key in seen:
            raise DuplicateKeyError(
                f"{path}: duplicate control key Tenant={row.tenant!r}, AccountID={row.account_id!r}."
            )
        seen.add(row.key)
        rows.append(row)
    if not rows:
        raise SchemaError(f"{path}: no trial-balance rows were supplied.")
    tenants = {row.tenant for row in rows}
    report_dates = {row.report_date for row in rows}
    if len(tenants) != 1:
        raise SchemaError(f"{path}: a canonical trial balance must contain exactly one tenant.")
    if len(report_dates) != 1:
        raise DateMismatchError(
            f"{path}: a canonical trial balance must contain exactly one ReportDate."
        )
    return rows


def load_open_items(snapshot: SourceSnapshot) -> list[OpenItem]:
    path = snapshot.path
    items: list[OpenItem] = []
    seen: set[str] = set()
    for row_number, values in _read_csv_rows(snapshot, OPEN_ITEM_COLUMNS, label="Open-items file"):
        item_id = _text(values["ItemID"], field="ItemID", row_number=row_number, path=path)
        if item_id in seen:
            raise DuplicateKeyError(f"{path}: duplicate ItemID {item_id!r}.")
        seen.add(item_id)
        severity = _text(values["Severity"], field="Severity", row_number=row_number, path=path)
        if severity not in OPEN_ITEM_SEVERITIES:
            raise SchemaError(
                f"{path}: row {row_number} Severity must be BLOCKING, EXPLAIN, or TRIVIAL."
            )
        status = _text(values["Status"], field="Status", row_number=row_number, path=path)
        if status not in OPEN_ITEM_STATUSES:
            raise SchemaError(f"{path}: row {row_number} Status must be OPEN or CLEARED.")
        due_date = parse_iso_date(
            _text(values["DueDate"], field="DueDate", row_number=row_number, path=path),
            field="DueDate",
            path=path,
            row_number=row_number,
        )
        items.append(
            OpenItem(
                item_id=item_id,
                severity=severity,  # type: ignore[arg-type]
                owner=_text(values["Owner"], field="Owner", row_number=row_number, path=path),
                due_date=due_date,
                status=status,  # type: ignore[arg-type]
                description=_text(
                    values["Description"], field="Description", row_number=row_number, path=path
                ),
                resolution=_text(
                    values["Resolution"],
                    field="Resolution",
                    row_number=row_number,
                    path=path,
                    allow_empty=True,
                ),
            )
        )
    return items


def load_activity_statement(snapshot: SourceSnapshot) -> list[ActivityLabel]:
    path = snapshot.path
    rows: list[ActivityLabel] = []
    seen: set[str] = set()
    for row_number, values in _read_csv_rows(
        snapshot, ACTIVITY_COLUMNS, label="Activity-statement file"
    ):
        label = _text(values["Label"], field="Label", row_number=row_number, path=path)
        if label in seen:
            raise DuplicateKeyError(f"{path}: duplicate Label {label!r}.")
        seen.add(label)
        rows.append(
            ActivityLabel(
                label=label,
                amount=parse_money(values["Amount"], field="Amount", row_number=row_number, path=path),
            )
        )
    if not rows:
        raise SchemaError(f"{path}: no activity-statement rows were supplied.")
    return rows


def load_gst_control(snapshot: SourceSnapshot) -> list[GstControlRow]:
    path = snapshot.path
    rows: list[GstControlRow] = []
    for row_number, values in _read_csv_rows(
        snapshot, GST_CONTROL_COLUMNS, label="GST-control file"
    ):
        rows.append(
            GstControlRow(
                posted_on=parse_iso_date(
                    _text(values["Date"], field="Date", row_number=row_number, path=path),
                    field="Date",
                    path=path,
                    row_number=row_number,
                ),
                account_id=_text(
                    values["AccountID"], field="AccountID", row_number=row_number, path=path
                ),
                account_name=_text(
                    values["AccountName"], field="AccountName", row_number=row_number, path=path
                ),
                debit=parse_money(values["Debit"], field="Debit", row_number=row_number, path=path),
                credit=parse_money(
                    values["Credit"], field="Credit", row_number=row_number, path=path
                ),
                description=_text(
                    values["Description"],
                    field="Description",
                    row_number=row_number,
                    path=path,
                    allow_empty=True,
                ),
            )
        )
    if not rows:
        raise SchemaError(f"{path}: no GST-control rows were supplied.")
    return rows


def load_bank_rec(snapshot: SourceSnapshot) -> list[BankRecRow]:
    path = snapshot.path
    rows: list[BankRecRow] = []
    seen: set[str] = set()
    for row_number, values in _read_csv_rows(snapshot, BANK_REC_COLUMNS, label="Bank-rec file"):
        account_id = _text(values["AccountID"], field="AccountID", row_number=row_number, path=path)
        if account_id in seen:
            raise DuplicateKeyError(f"{path}: duplicate AccountID {account_id!r}.")
        seen.add(account_id)
        rows.append(
            BankRecRow(
                account_id=account_id,
                statement_balance=parse_money(
                    values["StatementBalance"],
                    field="StatementBalance",
                    row_number=row_number,
                    path=path,
                ),
                gl_balance=parse_money(
                    values["GLBalance"], field="GLBalance", row_number=row_number, path=path
                ),
            )
        )
    if not rows:
        raise SchemaError(f"{path}: no bank-rec rows were supplied.")
    return rows


def load_tie_out_matrix(snapshot: SourceSnapshot) -> list[TieOutRow]:
    path = snapshot.path
    rows: list[TieOutRow] = []
    seen: set[str] = set()
    for row_number, values in _read_csv_rows(snapshot, TIE_OUT_COLUMNS, label="Tie-out file"):
        statement_line = _text(
            values["StatementLine"], field="StatementLine", row_number=row_number, path=path
        )
        if statement_line in seen:
            raise DuplicateKeyError(f"{path}: duplicate StatementLine {statement_line!r}.")
        seen.add(statement_line)
        status = _text(values["Status"], field="Status", row_number=row_number, path=path)
        if status not in TIE_OUT_STATUSES:
            raise SchemaError(
                f"{path}: row {row_number} Status must be TIED, EXCEPTION, or UNSUPPORTED."
            )
        rows.append(
            TieOutRow(
                statement_line=statement_line,
                statement_amount=parse_money(
                    values["StatementAmount"],
                    field="StatementAmount",
                    row_number=row_number,
                    path=path,
                ),
                workpaper_ref=_text(
                    values["WorkpaperRef"],
                    field="WorkpaperRef",
                    row_number=row_number,
                    path=path,
                    allow_empty=status != "TIED",
                ),
                source_file=_text(
                    values["SourceFile"],
                    field="SourceFile",
                    row_number=row_number,
                    path=path,
                    allow_empty=status != "TIED",
                ),
                status=status,  # type: ignore[arg-type]
            )
        )
    if not rows:
        raise SchemaError(f"{path}: no tie-out rows were supplied.")
    return rows


def load_prior_findings(snapshot: SourceSnapshot) -> list[PriorFinding]:
    path = snapshot.path
    rows: list[PriorFinding] = []
    seen: set[tuple[str, str]] = set()
    for row_number, values in _read_csv_rows(
        snapshot, PRIOR_FINDING_COLUMNS, label="Prior-findings file"
    ):
        code = _text(values["FindingCode"], field="FindingCode", row_number=row_number, path=path)
        slot = _text(values["Slot"], field="Slot", row_number=row_number, path=path)
        status = _text(values["Status"], field="Status", row_number=row_number, path=path)
        if status not in OPEN_ITEM_STATUSES:
            raise SchemaError(f"{path}: row {row_number} Status must be OPEN or CLEARED.")
        key = (code, slot)
        if key in seen:
            raise DuplicateKeyError(f"{path}: duplicate prior finding {code!r} / {slot!r}.")
        seen.add(key)
        rows.append(PriorFinding(finding_code=code, slot=slot, status=status))  # type: ignore[arg-type]
    return rows


def load_self_review(snapshot: SourceSnapshot, *, expected_profile: str) -> SelfReview:
    path = snapshot.path
    try:
        payload = json.loads(snapshot.text(label="Self-review file", encoding="utf-8-sig"))
    except json.JSONDecodeError as exc:
        raise SchemaError(f"{path}: self-review is not valid JSON.") from exc
    if not isinstance(payload, dict) or set(payload) != SELF_REVIEW_KEYS:
        raise SchemaError(
            f"{path}: self-review must contain exactly preparer_initials, prepared_on, "
            "engagement_type, period_end, and assertions."
        )
    initials = payload["preparer_initials"]
    prepared_on = payload["prepared_on"]
    engagement_type = payload["engagement_type"]
    period_end = payload["period_end"]
    assertions = payload["assertions"]
    if not isinstance(initials, str) or not initials.strip() or len(initials.strip()) > 12:
        raise SchemaError(
            f"{path}: preparer_initials must be a non-empty string of at most 12 characters."
        )
    if _has_control_or_format_character(initials):
        raise SchemaError(f"{path}: preparer_initials contains a control or formatting character.")
    if not isinstance(engagement_type, str) or engagement_type != expected_profile:
        raise SchemaError(
            f"{path}: engagement_type must match the selected profile {expected_profile!r}."
        )
    if not isinstance(prepared_on, str):
        raise SchemaError(f"{path}: prepared_on must be an ISO date string.")
    if not isinstance(period_end, str):
        raise SchemaError(f"{path}: period_end must be an ISO date string.")
    prepared_date = parse_iso_date(prepared_on, field="prepared_on", path=path)
    period_date = parse_iso_date(period_end, field="period_end", path=path)
    if prepared_date < period_date:
        raise SchemaError(
            f"{path}: prepared_on {prepared_date.isoformat()} is earlier than period_end "
            f"{period_date.isoformat()}."
        )
    if not isinstance(assertions, dict) or set(assertions) != set(SELF_REVIEW_ASSERTIONS):
        raise SchemaError(
            f"{path}: assertions must contain exactly {', '.join(SELF_REVIEW_ASSERTIONS)}."
        )
    typed: dict[str, bool] = {}
    for name in SELF_REVIEW_ASSERTIONS:
        value = assertions[name]
        if not isinstance(value, bool):
            raise SchemaError(f"{path}: assertions.{name} must be a JSON boolean.")
        typed[name] = value
    return SelfReview(
        preparer_initials=initials.strip(),
        prepared_on=prepared_date,
        engagement_type=engagement_type,
        period_end=period_date,
        assertions=typed,
    )


def load_reviewer_acknowledgement(path: Path | None) -> ReviewerAcknowledgement | None:
    if path is None:
        return None
    snapshot = SourceSnapshot.capture(path, label="Review-note file")
    try:
        payload = json.loads(snapshot.text(label="Review-note file", encoding="utf-8-sig"))
    except json.JSONDecodeError as exc:
        raise SchemaError(f"{path}: review note is not valid JSON.") from exc
    if not isinstance(payload, dict) or set(payload) != {
        "reviewer_initials",
        "reviewed_on",
        "comment",
    }:
        raise SchemaError(
            f"{path}: review note must contain exactly reviewer_initials, reviewed_on, and comment."
        )
    initials = payload["reviewer_initials"]
    comment = payload["comment"]
    reviewed_on = payload["reviewed_on"]
    if not isinstance(initials, str) or not initials.strip() or len(initials.strip()) > 12:
        raise SchemaError(
            f"{path}: reviewer_initials must be a non-empty string of at most 12 characters."
        )
    if not isinstance(comment, str) or not comment.strip():
        raise SchemaError(f"{path}: comment must be a non-empty string.")
    if _has_control_or_format_character(initials):
        raise SchemaError(f"{path}: reviewer_initials contains a control or formatting character.")
    if _has_control_or_format_character(comment, allow_line_breaks=True):
        raise SchemaError(f"{path}: comment contains a control or formatting character.")
    if not isinstance(reviewed_on, str):
        raise SchemaError(f"{path}: reviewed_on must be an ISO date string.")
    reviewed_date = parse_iso_date(reviewed_on, field="reviewed_on", path=path)
    return ReviewerAcknowledgement(initials.strip(), reviewed_date, comment.strip())
