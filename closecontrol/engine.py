from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal, DivisionByZero, InvalidOperation
from pathlib import Path

from .errors import DateMismatchError, SchemaError
from .loader import (
    SourceSnapshot,
    load_canonical_tb,
    load_mapping,
    load_reviewer_acknowledgement,
    load_subledger,
)
from .models import ExceptionItem, ReviewerAcknowledgement, Status, TrialBalanceRow


ZERO = Decimal("0")


@dataclass(frozen=True)
class CloseReviewPack:
    status: Status
    current_report_dates: tuple[str, ...]
    prior_report_dates: tuple[str, ...]
    source_hashes: dict[str, str]
    absolute_threshold: Decimal
    percentage_threshold: Decimal
    reconciliation_tolerance: Decimal
    exceptions: tuple[ExceptionItem, ...]
    acknowledgement: ReviewerAcknowledgement | None


def _percent_change(current: Decimal, prior: Decimal) -> Decimal | None:
    if prior == ZERO:
        return None
    try:
        return abs((current - prior) / prior)
    except (DivisionByZero, InvalidOperation):
        return None


def _exception(
    control: str,
    status: Status,
    row: TrialBalanceRow | None,
    *,
    current_value: Decimal | None = None,
    prior_value: Decimal | None = None,
    difference: Decimal | None = None,
    threshold: Decimal | None = None,
    percentage_change: Decimal | None = None,
    reason: str,
    reviewer_action: str,
) -> ExceptionItem:
    # One extraction for the row-or-blank fields instead of a per-field
    # conditional; behaviour is identical for both a present and an absent row.
    tenant, account_id, account_code, account_name = (
        (row.tenant, row.account_id, row.account_code, row.account_name)
        if row is not None
        else ("", "", "", "")
    )
    return ExceptionItem(
        control=control,
        status=status,
        tenant=tenant,
        account_id=account_id,
        account_code=account_code,
        account_name=account_name,
        current_value=current_value,
        prior_value=prior_value,
        difference=difference,
        threshold=threshold,
        percentage_change=percentage_change,
        reason=reason,
        reviewer_action=reviewer_action,
    )


def _integrity_exceptions(rows: list[TrialBalanceRow], period: str) -> list[ExceptionItem]:
    debit_total = sum((row.debit for row in rows), ZERO)
    credit_total = sum((row.credit for row in rows), ZERO)
    ytd_debit_total = sum((row.ytd_debit for row in rows), ZERO)
    ytd_credit_total = sum((row.ytd_credit for row in rows), ZERO)
    result: list[ExceptionItem] = []
    if debit_total != credit_total:
        result.append(
            _exception(
                "trial_balance_integrity", "BLOCKED", None,
                current_value=debit_total,
                prior_value=credit_total,
                difference=debit_total - credit_total,
                threshold=ZERO,
                reason=f"{period} movement debit and credit totals do not balance.",
                reviewer_action="Correct or re-export the source trial balance before relying on any review result.",
            )
        )
    if ytd_debit_total != ytd_credit_total:
        result.append(
            _exception(
                "trial_balance_integrity", "BLOCKED", None,
                current_value=ytd_debit_total,
                prior_value=ytd_credit_total,
                difference=ytd_debit_total - ytd_credit_total,
                threshold=ZERO,
                reason=f"{period} YTD debit and credit totals do not balance.",
                reviewer_action="Correct or re-export the source trial balance before relying on any review result.",
            )
        )
    return result


def _period_comparison_exceptions(
    current_by_key: dict[tuple[str, str], TrialBalanceRow],
    prior_by_key: dict[tuple[str, str], TrialBalanceRow],
    absolute_threshold: Decimal,
    percentage_threshold: Decimal,
) -> list[ExceptionItem]:
    result: list[ExceptionItem] = []
    for key in sorted(set(current_by_key) | set(prior_by_key)):
        current = current_by_key.get(key)
        prior = prior_by_key.get(key)
        display_row = current or prior
        if display_row is None:  # Defensive: key came from one of the two maps.
            raise RuntimeError("period comparison produced a key without a source row")
        if current is None:
            result.append(
                _exception(
                    "period_comparison", "REVIEW", display_row,
                    current_value=None,
                    prior_value=prior.ytd_net if prior else None,
                    difference=None,
                    reason="Account was present in the prior period but is absent from the current period.",
                    reviewer_action="Confirm the account was closed, reclassified, or omitted intentionally.",
                )
            )
            continue
        if prior is None:
            result.append(
                _exception(
                    "period_comparison", "REVIEW", current,
                    current_value=current.ytd_net,
                    prior_value=None,
                    difference=None,
                    reason="New account is present in the current period only.",
                    reviewer_action="Confirm the account setup, mapping, and first-period activity.",
                )
            )
        else:
            difference = current.ytd_net - prior.ytd_net
            percentage = _percent_change(current.ytd_net, prior.ytd_net)
            material_by_amount = difference != ZERO and abs(difference) >= absolute_threshold
            # A nil prior balance leaves no percentage change to compute, so the
            # percentage gate cannot be applied and the absolute gate decides
            # alone. The exception says which gates were actually tested: the
            # two-threshold wording alongside a blank percentage_change column
            # would tell the reviewer a percentage test passed that never ran.
            # The wording follows `percentage is None` rather than the nil prior
            # balance itself, so it stays true for the defensive None that
            # _percent_change returns if the division cannot be represented.
            material_by_percentage = percentage is None or percentage >= percentage_threshold
            if material_by_amount and material_by_percentage:
                result.append(
                    _exception(
                        "period_variance", "REVIEW", current,
                        current_value=current.ytd_net,
                        prior_value=prior.ytd_net,
                        difference=difference,
                        threshold=absolute_threshold,
                        percentage_change=percentage,
                        reason=(
                            "YTD net balance moved beyond both configured materiality thresholds."
                            if percentage is not None
                            else "YTD net balance moved beyond the configured absolute threshold; no percentage change could be computed from the prior YTD balance, so the percentage threshold was not tested."
                        ),
                        reviewer_action="Investigate the driver, retain supporting evidence, and document the reviewer conclusion.",
                    )
                )
            changed = []
            if current.account_code != prior.account_code:
                changed.append("account code")
            if current.account_name != prior.account_name:
                changed.append("account name")
            if current.section != prior.section:
                changed.append("section")
            if changed:
                result.append(
                    _exception(
                        "account_metadata", "REVIEW", current,
                        current_value=current.ytd_net,
                        prior_value=prior.ytd_net,
                        difference=difference,
                        reason=f"Stable AccountID has changed {' and '.join(changed)} since the prior period.",
                        reviewer_action="Confirm that the chart-of-accounts change and any reporting impact were reviewed.",
                    )
                )
    return result


def _mapping_exceptions(rows: list[TrialBalanceRow], mapping: dict[str, str]) -> list[ExceptionItem]:
    result: list[ExceptionItem] = []
    for row in sorted(rows, key=lambda item: item.key):
        if row.account_id not in mapping:
            result.append(
                _exception(
                    "account_mapping", "REVIEW", row,
                    current_value=row.ytd_net,
                    reason="Current account has no supplied review-group mapping.",
                    reviewer_action="Assign and review an appropriate reporting group before using a grouped close pack.",
                )
            )
    return result


def _subledger_exceptions(
    current_by_key: dict[tuple[str, str], TrialBalanceRow],
    subledger: dict[tuple[str, str], Decimal],
    reconciliation_tolerance: Decimal,
) -> list[ExceptionItem]:
    result: list[ExceptionItem] = []
    for key, subledger_value in sorted(subledger.items()):
        current = current_by_key.get(key)
        if current is None:
            tenant, account_id = key
            result.append(
                ExceptionItem(
                    control="subledger_reconciliation",
                    status="REVIEW",
                    tenant=tenant,
                    account_id=account_id,
                    account_code="",
                    account_name="",
                    current_value=None,
                    prior_value=subledger_value,
                    difference=None,
                    threshold=reconciliation_tolerance,
                    percentage_change=None,
                    reason="Supplied subledger balance has no matching current trial-balance account.",
                    reviewer_action="Confirm the control-account mapping or correct the source files.",
                )
            )
            continue
        difference = current.ytd_net - subledger_value
        if abs(difference) > reconciliation_tolerance:
            result.append(
                _exception(
                    "subledger_reconciliation", "REVIEW", current,
                    current_value=current.ytd_net,
                    prior_value=subledger_value,
                    difference=difference,
                    threshold=reconciliation_tolerance,
                    reason="Current trial-balance balance differs from the supplied subledger beyond tolerance.",
                    reviewer_action="Reconcile the difference to timing, mapping, or source records and retain support.",
                )
            )
    return result


def _overall_status(exceptions: list[ExceptionItem]) -> Status:
    if any(item.status == "BLOCKED" for item in exceptions):
        return "BLOCKED"
    if any(item.status == "REVIEW" for item in exceptions):
        return "REVIEW"
    return "PASS"


def review_close(
    *,
    current_path: Path,
    prior_path: Path,
    mapping_path: Path | None = None,
    subledger_path: Path | None = None,
    acknowledgement_path: Path | None = None,
    absolute_threshold: Decimal = Decimal("1000"),
    percentage_threshold: Decimal = Decimal("0.10"),
    reconciliation_tolerance: Decimal = Decimal("0.01"),
) -> CloseReviewPack:
    """Create a deterministic review pack without mutating any accounting system."""
    for name, value in {
        "absolute_threshold": absolute_threshold,
        "percentage_threshold": percentage_threshold,
        "reconciliation_tolerance": reconciliation_tolerance,
    }.items():
        if not value.is_finite() or value < ZERO:
            raise ValueError(f"{name} must be a finite non-negative decimal.")

    # Each source is read once. Parsing and provenance use the same immutable
    # bytes, so replacing a file while the review runs cannot make calculations
    # from one version travel with the digest of another.
    current_source = SourceSnapshot.capture(current_path, label="Trial-balance file")
    current_rows = load_canonical_tb(current_source)
    prior_source = SourceSnapshot.capture(prior_path, label="Trial-balance file")
    prior_rows = load_canonical_tb(prior_source)
    mapping_source = (
        SourceSnapshot.capture(mapping_path, label="Mapping file")
        if mapping_path is not None
        else None
    )
    mapping = load_mapping(mapping_source)
    subledger_source = (
        SourceSnapshot.capture(subledger_path, label="Subledger file")
        if subledger_path is not None
        else None
    )
    subledger = load_subledger(subledger_source)
    acknowledgement_source = (
        SourceSnapshot.capture(acknowledgement_path, label="Review-note file")
        if acknowledgement_path is not None
        else None
    )
    acknowledgement = load_reviewer_acknowledgement(acknowledgement_source)

    current_tenant = current_rows[0].tenant
    prior_tenant = prior_rows[0].tenant
    current_date = current_rows[0].report_date
    prior_date = prior_rows[0].report_date
    if current_tenant != prior_tenant:
        raise SchemaError(
            "Current and prior trial balances must contain the same tenant."
        )
    if prior_date >= current_date:
        raise DateMismatchError(
            "Prior trial-balance ReportDate must be earlier than the current ReportDate."
        )
    if acknowledgement is not None and acknowledgement.reviewed_on < current_date:
        raise DateMismatchError(
            "Review-note reviewed_on cannot be earlier than the current ReportDate."
        )

    current_by_key = {row.key: row for row in current_rows}
    prior_by_key = {row.key: row for row in prior_rows}
    exceptions = _integrity_exceptions(current_rows, "Current") + _integrity_exceptions(prior_rows, "Prior")
    exceptions += _period_comparison_exceptions(
        current_by_key, prior_by_key, absolute_threshold, percentage_threshold
    )
    # Only a supplied mapping file makes the mapping control run: with no file
    # every account would be an exception against an empty mapping.
    if mapping_path is not None:
        exceptions += _mapping_exceptions(current_rows, mapping)
    exceptions += _subledger_exceptions(current_by_key, subledger, reconciliation_tolerance)

    source_hashes = {
        "current_trial_balance": current_source.sha256,
        "prior_trial_balance": prior_source.sha256,
    }
    if mapping_source is not None:
        source_hashes["account_mapping"] = mapping_source.sha256
    if subledger_source is not None:
        source_hashes["subledger"] = subledger_source.sha256
    if acknowledgement_source is not None:
        source_hashes["review_note"] = acknowledgement_source.sha256

    ordered = tuple(sorted(exceptions, key=lambda item: (item.status != "BLOCKED", item.control, item.tenant, item.account_id, item.reason)))
    return CloseReviewPack(
        status=_overall_status(list(ordered)),
        current_report_dates=tuple(sorted({row.report_date.isoformat() for row in current_rows})),
        prior_report_dates=tuple(sorted({row.report_date.isoformat() for row in prior_rows})),
        source_hashes=source_hashes,
        absolute_threshold=absolute_threshold,
        percentage_threshold=percentage_threshold,
        reconciliation_tolerance=reconciliation_tolerance,
        exceptions=ordered,
        acknowledgement=acknowledgement,
    )
