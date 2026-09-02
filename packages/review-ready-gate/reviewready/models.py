from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from decimal import Decimal
from typing import Literal


Status = Literal["READY", "NOT_READY", "BLOCKED"]
FindingStatus = Literal["NOT_READY", "BLOCKED"]
Severity = Literal["BLOCKING", "EXPLAIN", "TRIVIAL"]
OpenItemStatus = Literal["OPEN", "CLEARED"]
TieOutStatus = Literal["TIED", "EXCEPTION", "UNSUPPORTED"]

FINDING_MISSING_ARTEFACT = "MISSING_ARTEFACT"
FINDING_EMPTY_ARTEFACT = "EMPTY_ARTEFACT"
FINDING_TB_UNBALANCED = "TB_UNBALANCED"
FINDING_OPEN_ITEM_INCOMPLETE = "OPEN_ITEM_INCOMPLETE"
FINDING_OPEN_ITEM_BLOCKING = "OPEN_ITEM_BLOCKING"
FINDING_SELF_REVIEW_INCOMPLETE = "SELF_REVIEW_INCOMPLETE"
FINDING_TIEOUT_BREAK = "TIEOUT_BREAK"
FINDING_BANK_REC_BREAK = "BANK_REC_BREAK"
FINDING_TIEOUT_UNSUPPORTED = "TIEOUT_UNSUPPORTED"
FINDING_PERIOD_ORDER = "PERIOD_ORDER"


@dataclass(frozen=True)
class TrialBalanceRow:
    report_date: date
    tenant: str
    section: str
    account_id: str
    account_name: str
    account_code: str
    debit: Decimal
    credit: Decimal
    ytd_debit: Decimal
    ytd_credit: Decimal

    @property
    def key(self) -> tuple[str, str]:
        return (self.tenant, self.account_id)


@dataclass(frozen=True)
class OpenItem:
    item_id: str
    severity: Severity
    owner: str
    due_date: date
    status: OpenItemStatus
    description: str
    resolution: str


@dataclass(frozen=True)
class SelfReview:
    preparer_initials: str
    prepared_on: date
    engagement_type: str
    period_end: date
    assertions: dict[str, bool]


@dataclass(frozen=True)
class ActivityLabel:
    label: str
    amount: Decimal


@dataclass(frozen=True)
class GstControlRow:
    posted_on: date
    account_id: str
    account_name: str
    debit: Decimal
    credit: Decimal
    description: str


@dataclass(frozen=True)
class BankRecRow:
    account_id: str
    statement_balance: Decimal
    gl_balance: Decimal


@dataclass(frozen=True)
class TieOutRow:
    statement_line: str
    statement_amount: Decimal
    workpaper_ref: str
    source_file: str
    status: TieOutStatus


@dataclass(frozen=True)
class PriorFinding:
    finding_code: str
    slot: str
    status: OpenItemStatus


@dataclass(frozen=True)
class Finding:
    code: str
    status: FindingStatus
    slot: str
    reason: str
    reviewer_action: str
    repeat: bool = False


@dataclass(frozen=True)
class ReviewerAcknowledgement:
    reviewer_initials: str
    reviewed_on: date
    comment: str


@dataclass(frozen=True)
class SourceEvidence:
    slot: str
    filename: str
    sha256: str
