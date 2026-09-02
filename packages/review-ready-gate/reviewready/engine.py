from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path

from .errors import GateInputError
from .loader import (
    SourceSnapshot,
    load_activity_statement,
    load_bank_rec,
    load_canonical_tb,
    load_gst_control,
    load_open_items,
    load_prior_findings,
    load_reviewer_acknowledgement,
    load_self_review,
    load_tie_out_matrix,
)
from .models import (
    FINDING_BANK_REC_BREAK,
    FINDING_EMPTY_ARTEFACT,
    FINDING_MISSING_ARTEFACT,
    FINDING_OPEN_ITEM_BLOCKING,
    FINDING_OPEN_ITEM_INCOMPLETE,
    FINDING_PERIOD_ORDER,
    FINDING_SELF_REVIEW_INCOMPLETE,
    FINDING_TB_UNBALANCED,
    FINDING_TIEOUT_BREAK,
    FINDING_TIEOUT_UNSUPPORTED,
    ActivityLabel,
    BankRecRow,
    Finding,
    GstControlRow,
    OpenItem,
    ReviewerAcknowledgement,
    SelfReview,
    SourceEvidence,
    Status,
    TieOutRow,
    TrialBalanceRow,
)
from .profiles import Slot, slots_for


@dataclass(frozen=True)
class ReadinessPack:
    status: Status
    engagement_type: str
    period_end: str
    preparer_initials: str
    findings: tuple[Finding, ...]
    source_evidence: tuple[SourceEvidence, ...]
    tieout_tolerance: Decimal
    acknowledgement: ReviewerAcknowledgement | None


def _missing(slot: str) -> Finding:
    return Finding(
        code=FINDING_MISSING_ARTEFACT,
        status="NOT_READY",
        slot=slot,
        reason=f"Required artefact {slot} is not in the pack directory.",
        reviewer_action="Return the pack to the preparer. Do not start technical review.",
    )


def _empty(slot: str) -> Finding:
    return Finding(
        code=FINDING_EMPTY_ARTEFACT,
        status="NOT_READY",
        slot=slot,
        reason=f"Required artefact {slot} exists but contains no bytes.",
        reviewer_action="Return the pack to the preparer. An empty file is not a completed artefact.",
    )


def _tb_balanced(rows: list[TrialBalanceRow]) -> tuple[Decimal, Decimal]:
    movement = sum((row.debit - row.credit for row in rows), Decimal("0.00"))
    ytd = sum((row.ytd_debit - row.ytd_credit for row in rows), Decimal("0.00"))
    return movement, ytd


def _mark_repeats(
    findings: list[Finding], prior: list[tuple[str, str]]
) -> list[Finding]:
    open_prior = set(prior)
    marked: list[Finding] = []
    for finding in findings:
        repeat = (finding.code, finding.slot) in open_prior
        if repeat and not finding.repeat:
            finding = Finding(
                code=finding.code,
                status=finding.status,
                slot=finding.slot,
                reason=finding.reason + " This finding was OPEN on the prior pack.",
                reviewer_action=finding.reviewer_action
                + " Treat as a repeated preparer or vendor failure.",
                repeat=True,
            )
        marked.append(finding)
    return marked


def _overall(findings: list[Finding]) -> Status:
    if any(item.status == "BLOCKED" for item in findings):
        return "BLOCKED"
    if findings:
        return "NOT_READY"
    return "READY"


def review_pack(
    *,
    profile: str,
    pack_dir: Path,
    acknowledgement_path: Path | None = None,
    tieout_tolerance: Decimal = Decimal("0.01"),
) -> ReadinessPack:
    if not tieout_tolerance.is_finite() or tieout_tolerance < 0:
        raise GateInputError("tie-out tolerance must be a finite non-negative decimal.")
    try:
        slots = slots_for(profile)
    except KeyError as exc:
        raise GateInputError(str(exc)) from exc
    if not pack_dir.is_dir():
        raise GateInputError(f"pack directory does not exist: {pack_dir}.")

    findings: list[Finding] = []
    evidence: list[SourceEvidence] = []
    snapshots: dict[str, SourceSnapshot] = {}
    loaded: dict[str, object] = {}

    for slot in slots:
        path = pack_dir / slot.filename
        if not path.exists():
            if slot.required:
                findings.append(_missing(slot.name))
            continue
        snapshot = SourceSnapshot.capture(path, label=f"{slot.name} file")
        snapshots[slot.name] = snapshot
        evidence.append(
            SourceEvidence(slot=slot.name, filename=slot.filename, sha256=snapshot.sha256)
        )
        if len(snapshot.content) == 0:
            if slot.required:
                findings.append(_empty(slot.name))
            continue
        loaded[slot.name] = _load_slot(slot, snapshot, profile)

    self_review = loaded.get("self_review")
    period_end = ""
    preparer_initials = ""
    if isinstance(self_review, SelfReview):
        period_end = self_review.period_end.isoformat()
        preparer_initials = self_review.preparer_initials
        incomplete = [name for name, value in self_review.assertions.items() if value is not True]
        if incomplete:
            findings.append(
                Finding(
                    code=FINDING_SELF_REVIEW_INCOMPLETE,
                    status="NOT_READY",
                    slot="self_review",
                    reason="Self-review assertions still false: " + ", ".join(incomplete) + ".",
                    reviewer_action="Return the pack. The preparer has not certified it as review-ready.",
                )
            )

    _apply_trial_balance_controls(loaded, findings)
    _apply_open_item_controls(loaded, findings)
    _apply_bas_tieout(loaded, findings, tieout_tolerance)
    _apply_bank_rec(loaded, findings, tieout_tolerance)
    _apply_year_end_tieout(loaded, findings)

    prior = loaded.get("prior_findings")
    if isinstance(prior, list):
        open_prior = [(row.finding_code, row.slot) for row in prior if row.status == "OPEN"]
        findings = _mark_repeats(findings, open_prior)

    acknowledgement = load_reviewer_acknowledgement(acknowledgement_path)
    if acknowledgement is not None and isinstance(self_review, SelfReview):
        if acknowledgement.reviewed_on < self_review.period_end:
            raise GateInputError(
                "review note reviewed_on is earlier than the pack period_end; "
                "a note cannot review a period that has not ended."
            )

    # Acknowledgement never changes status.
    status = _overall(findings)
    return ReadinessPack(
        status=status,
        engagement_type=profile,
        period_end=period_end,
        preparer_initials=preparer_initials,
        findings=tuple(findings),
        source_evidence=tuple(evidence),
        tieout_tolerance=tieout_tolerance,
        acknowledgement=acknowledgement,
    )


def _load_slot(slot: Slot, snapshot: SourceSnapshot, profile: str) -> object:
    if slot.kind == "trial_balance":
        return load_canonical_tb(snapshot)
    if slot.kind == "open_items":
        return load_open_items(snapshot)
    if slot.kind == "self_review":
        return load_self_review(snapshot, expected_profile=profile)
    if slot.kind == "activity_statement":
        return load_activity_statement(snapshot)
    if slot.kind == "gst_control_gl":
        return load_gst_control(snapshot)
    if slot.kind == "bank_rec":
        return load_bank_rec(snapshot)
    if slot.kind == "tie_out_matrix":
        return load_tie_out_matrix(snapshot)
    if slot.kind == "prior_findings":
        return load_prior_findings(snapshot)
    raise GateInputError(f"unknown slot kind {slot.kind!r}.")


def _apply_trial_balance_controls(loaded: dict[str, object], findings: list[Finding]) -> None:
    current = loaded.get("trial_balance")
    if isinstance(current, list) and current and isinstance(current[0], TrialBalanceRow):
        movement, ytd = _tb_balanced(current)
        if movement != 0 or ytd != 0:
            findings.append(
                Finding(
                    code=FINDING_TB_UNBALANCED,
                    status="BLOCKED",
                    slot="trial_balance",
                    reason=(
                        f"Trial balance does not balance. Movement net {movement} "
                        f"and YTD net {ytd} must both be zero."
                    ),
                    reviewer_action="Do not review judgement. Return the pack until the trial balance balances.",
                )
            )
    prior = loaded.get("prior_trial_balance")
    if isinstance(prior, list) and prior and isinstance(prior[0], TrialBalanceRow):
        movement, ytd = _tb_balanced(prior)
        if movement != 0 or ytd != 0:
            findings.append(
                Finding(
                    code=FINDING_TB_UNBALANCED,
                    status="BLOCKED",
                    slot="prior_trial_balance",
                    reason=(
                        f"Prior trial balance does not balance. Movement net {movement} "
                        f"and YTD net {ytd} must both be zero."
                    ),
                    reviewer_action="Return the pack. A prior period that does not balance cannot support comparative review.",
                )
            )
        if (
            isinstance(current, list)
            and current
            and isinstance(current[0], TrialBalanceRow)
        ):
            current_row = current[0]
            prior_row = prior[0]
            if current_row.tenant != prior_row.tenant:
                findings.append(
                    Finding(
                        code=FINDING_PERIOD_ORDER,
                        status="BLOCKED",
                        slot="prior_trial_balance",
                        reason=(
                            f"Current tenant {current_row.tenant!r} does not match "
                            f"prior tenant {prior_row.tenant!r}."
                        ),
                        reviewer_action="Return the pack. Comparative files must name the same entity.",
                    )
                )
            elif prior_row.report_date >= current_row.report_date:
                findings.append(
                    Finding(
                        code=FINDING_PERIOD_ORDER,
                        status="BLOCKED",
                        slot="prior_trial_balance",
                        reason=(
                            f"Prior ReportDate {prior_row.report_date.isoformat()} is not earlier than "
                            f"current ReportDate {current_row.report_date.isoformat()}."
                        ),
                        reviewer_action="Return the pack. The prior file must be an earlier period.",
                    )
                )


def _apply_open_item_controls(loaded: dict[str, object], findings: list[Finding]) -> None:
    items = loaded.get("open_items")
    if not isinstance(items, list):
        return
    for item in items:
        if not isinstance(item, OpenItem):
            continue
        if item.status == "OPEN" and item.severity == "BLOCKING" and not item.resolution:
            findings.append(
                Finding(
                    code=FINDING_OPEN_ITEM_BLOCKING,
                    status="NOT_READY",
                    slot="open_items",
                    reason=(
                        f"Blocking open item {item.item_id} still OPEN with no resolution: "
                        f"{item.description}"
                    ),
                    reviewer_action="Return the pack. Blocking items must be cleared or resolved before review.",
                )
            )
        if item.status == "CLEARED" and not item.resolution:
            findings.append(
                Finding(
                    code=FINDING_OPEN_ITEM_INCOMPLETE,
                    status="NOT_READY",
                    slot="open_items",
                    reason=f"Open item {item.item_id} is CLEARED but has no resolution text.",
                    reviewer_action="Return the pack. A cleared item needs a written resolution.",
                )
            )


def _apply_bas_tieout(
    loaded: dict[str, object], findings: list[Finding], tolerance: Decimal
) -> None:
    activity = loaded.get("activity_statement")
    gst = loaded.get("gst_control_gl")
    if not isinstance(activity, list) or not isinstance(gst, list):
        return
    labels = {row.label: row.amount for row in activity if isinstance(row, ActivityLabel)}
    if "1A" not in labels or "1B" not in labels:
        findings.append(
            Finding(
                code=FINDING_TIEOUT_BREAK,
                status="NOT_READY",
                slot="activity_statement",
                reason="Activity statement is missing label 1A or 1B, so net GST cannot be tied.",
                reviewer_action="Return the pack. BAS readiness requires 1A and 1B.",
            )
        )
        return
    net_statement = labels["1A"] - labels["1B"]
    net_gl = sum(
        (row.credit - row.debit for row in gst if isinstance(row, GstControlRow)),
        Decimal("0.00"),
    )
    difference = net_statement - net_gl
    if abs(difference) > tolerance:
        findings.append(
            Finding(
                code=FINDING_TIEOUT_BREAK,
                status="NOT_READY",
                slot="gst_control_gl",
                reason=(
                    f"Net GST on the activity statement ({net_statement}) does not tie to "
                    f"GST control movement ({net_gl}); difference {difference} exceeds "
                    f"tolerance {tolerance}."
                ),
                reviewer_action="Return the pack. Complete the GST control tie-out before review.",
            )
        )


def _apply_bank_rec(
    loaded: dict[str, object], findings: list[Finding], tolerance: Decimal
) -> None:
    recs = loaded.get("bank_rec")
    if not isinstance(recs, list):
        return
    for row in recs:
        if not isinstance(row, BankRecRow):
            continue
        difference = row.statement_balance - row.gl_balance
        if abs(difference) > tolerance:
            findings.append(
                Finding(
                    code=FINDING_BANK_REC_BREAK,
                    status="NOT_READY",
                    slot="bank_rec",
                    reason=(
                        f"Account {row.account_id} statement {row.statement_balance} "
                        f"differs from GL {row.gl_balance} by {difference}."
                    ),
                    reviewer_action="Return the pack. Finish the bank reconciliation before review.",
                )
            )


def _apply_year_end_tieout(loaded: dict[str, object], findings: list[Finding]) -> None:
    matrix = loaded.get("tie_out_matrix")
    if not isinstance(matrix, list):
        return
    for row in matrix:
        if not isinstance(row, TieOutRow):
            continue
        if row.status == "UNSUPPORTED":
            findings.append(
                Finding(
                    code=FINDING_TIEOUT_UNSUPPORTED,
                    status="NOT_READY",
                    slot="tie_out_matrix",
                    reason=(
                        f"Statement line {row.statement_line!r} is UNSUPPORTED; "
                        "there is no source for the figure."
                    ),
                    reviewer_action="Return the pack. Unsupported lines are not review-ready.",
                )
            )
        elif row.status == "TIED" and (not row.workpaper_ref or not row.source_file):
            findings.append(
                Finding(
                    code=FINDING_TIEOUT_UNSUPPORTED,
                    status="NOT_READY",
                    slot="tie_out_matrix",
                    reason=(
                        f"Statement line {row.statement_line!r} is marked TIED without "
                        "a workpaper reference and source file."
                    ),
                    reviewer_action="Return the pack. A tied line must name its evidence.",
                )
            )
