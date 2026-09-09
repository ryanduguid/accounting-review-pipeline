from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from decimal import Decimal
from typing import Literal


Status = Literal["PASS", "REVIEW", "BLOCKED"]


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

    @property
    def current_net(self) -> Decimal:
        return self.debit - self.credit

    @property
    def ytd_net(self) -> Decimal:
        return self.ytd_debit - self.ytd_credit


@dataclass(frozen=True)
class ExceptionItem:
    control: str
    status: Status
    tenant: str
    account_id: str
    account_code: str
    account_name: str
    current_value: Decimal | None
    prior_value: Decimal | None
    difference: Decimal | None
    threshold: Decimal | None
    percentage_change: Decimal | None
    reason: str
    reviewer_action: str
    # The ReviewGroup from the optional --mapping file for this exception's
    # account; blank when no mapping was supplied or the account is unmapped.
    review_group: str = ""


@dataclass(frozen=True)
class ClientQuery:
    """One question for the client, derived from an exception.

    The register carries no answer or status column. A pack is evidence of one
    run and `close-control view` proves its files still agree, so an answer
    written back into the file would invalidate the pack that raised the
    question. Answers belong in the firm's own tracker.
    """

    query_id: str
    control: str
    tenant: str
    account_id: str
    account_code: str
    account_name: str
    review_group: str
    difference: Decimal | None
    question: str
    evidence_requested: str


@dataclass(frozen=True)
class ReviewerAcknowledgement:
    reviewer_initials: str
    reviewed_on: date
    comment: str
