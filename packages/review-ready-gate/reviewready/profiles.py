from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Slot:
    name: str
    filename: str
    required: bool
    kind: str


_OPEN_ITEMS = Slot("open_items", "open_items.csv", True, "open_items")
_SELF_REVIEW = Slot("self_review", "self_review.json", True, "self_review")
_PRIOR_FINDINGS = Slot("prior_findings", "prior_findings.csv", False, "prior_findings")

PROFILES: dict[str, tuple[Slot, ...]] = {
    "bas": (
        Slot("trial_balance", "trial_balance.csv", True, "trial_balance"),
        Slot("activity_statement", "activity_statement.csv", True, "activity_statement"),
        Slot("gst_control_gl", "gst_control_gl.csv", True, "gst_control_gl"),
        _OPEN_ITEMS,
        _SELF_REVIEW,
        _PRIOR_FINDINGS,
    ),
    "month_end": (
        Slot("trial_balance", "trial_balance.csv", True, "trial_balance"),
        Slot("prior_trial_balance", "prior_trial_balance.csv", True, "trial_balance"),
        Slot("bank_rec", "bank_rec.csv", False, "bank_rec"),
        _OPEN_ITEMS,
        _SELF_REVIEW,
        _PRIOR_FINDINGS,
    ),
    "year_end": (
        Slot("trial_balance", "trial_balance.csv", True, "trial_balance"),
        Slot("prior_trial_balance", "prior_trial_balance.csv", True, "trial_balance"),
        Slot("tie_out_matrix", "tie_out_matrix.csv", True, "tie_out_matrix"),
        _OPEN_ITEMS,
        _SELF_REVIEW,
        _PRIOR_FINDINGS,
    ),
}

PROFILE_NAMES = tuple(PROFILES)


def slots_for(profile: str) -> tuple[Slot, ...]:
    try:
        return PROFILES[profile]
    except KeyError as exc:
        known = ", ".join(PROFILE_NAMES)
        raise KeyError(f"unknown engagement profile {profile!r}; expected one of: {known}.") from exc
