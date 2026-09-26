"""Compare two verified readiness packs for the same engagement and period.

A pack sent back to the preparer is gated again. This module verifies both runs
with the same fail-closed reader as ``review-ready view`` and reports what moved
between them: the overall status, each source file's digest and each group of
findings. It writes nothing and decides nothing.
"""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Any

from .errors import GateInputError
from .viewer import verify_pack

COMPARISON_SCHEMA_VERSION = 1

COMPARISON_BOUNDARY = (
    "Comparison is a review aid. It does not approve a file, change a computed "
    "status or record who changed what. Packs carry no wall-clock time, so the "
    "reviewer records who made each change and why."
)

# The finding members that belong to one finding rather than to its group key.
_FINDING_DETAIL = ("status", "repeat", "reason", "reviewer_action")


def _finding_groups(document: dict[str, Any]) -> dict[tuple[str, str], list[dict[str, str]]]:
    """Group findings by code and slot, keeping every finding in each group."""
    groups: dict[tuple[str, str], list[dict[str, str]]] = {}
    for item in document["findings"]:
        detail = {member: item[member] for member in _FINDING_DETAIL}
        groups.setdefault((item["code"], item["slot"]), []).append(detail)
    return groups


def _canonical(group: list[dict[str, str]]) -> list[tuple[str, ...]]:
    """Order-insensitive form of a group, so a reordered run still recurs."""
    return sorted(tuple(detail[member] for member in _FINDING_DETAIL) for detail in group)


def _thresholds_differ(previous: dict[str, Any], current: dict[str, Any]) -> bool:
    """Compare tolerances by value: 0.01 and 0.010 are one setting written two ways.

    ``verify_pack`` has already proved every threshold is a finite decimal string.
    """
    def values(document: dict[str, Any]) -> dict[str, Decimal]:
        return {key: Decimal(text) for key, text in document["thresholds"].items()}

    return values(previous) != values(current)


def _scope_changes(previous: dict[str, Any], current: dict[str, Any]) -> list[dict[str, Any]]:
    """Name every setting that changes what a run could find."""
    changes = []
    if _thresholds_differ(previous, current):
        changes.append({
            "member": "thresholds",
            "previous": previous["thresholds"],
            "current": current["thresholds"],
        })
    before, after = previous.get("controls_not_run"), current.get("controls_not_run")
    if before != after:
        changes.append({"member": "controls_not_run", "previous": before, "current": after})
    return changes


def _source_changes(previous: dict[str, Any], current: dict[str, Any]) -> list[dict[str, Any]]:
    before, after = previous["source_sha256"], current["source_sha256"]
    rows = []
    for slot in sorted(set(before) | set(after)):
        if slot not in before:
            change = "ADDED"
        elif slot not in after:
            change = "REMOVED"
        elif before[slot] != after[slot]:
            change = "CHANGED"
        else:
            change = "UNCHANGED"
        rows.append({
            "slot": slot,
            "change": change,
            "previous": before.get(slot),
            "current": after.get(slot),
        })
    return rows


def _finding_changes(previous: dict[str, Any], current: dict[str, Any]) -> list[dict[str, Any]]:
    before, after = _finding_groups(previous), _finding_groups(current)
    thresholds_changed = _thresholds_differ(previous, current)
    # An omitted member is a pack that states no coverage, not one that ran everything.
    coverage = current.get("controls_not_run")
    not_run = {control["slot"] for control in coverage or ()}
    rows = []
    for code, slot in sorted(set(before) | set(after)):
        old, new = before.get((code, slot)), after.get((code, slot))
        if old is None:
            change = "NEW"
        elif new is None:
            # Absent from the later run is not resolved. Under a changed
            # tolerance, when the later run did not check that slot, or when
            # it states no coverage at all, it is not even comparable.
            comparable = not thresholds_changed and coverage is not None and slot not in not_run
            change = "NOT_RAISED" if comparable else "NOT_COMPARABLE"
        elif _canonical(old) == _canonical(new):
            change = "RECURRING"
        else:
            change = "CHANGED"
        rows.append({
            "code": code,
            "slot": slot,
            "change": change,
            "previous": old or [],
            "current": new or [],
        })
    return rows


def compare_packs(previous_dir: Path, current_dir: Path) -> dict[str, Any]:
    """Verify both packs, then describe what moved from the first to the second.

    Raises ``GateInputError`` when either pack fails verification or the two
    packs describe a different engagement type or period.
    """
    previous, _, _, previous_artefacts = verify_pack(previous_dir)
    current, _, _, current_artefacts = verify_pack(current_dir)
    for member in ("engagement_type", "period_end"):
        if previous[member] != current[member]:
            label = member.replace("_", " ")
            raise GateInputError(
                f"compare needs two runs of the same {label}; the previous pack has "
                f"{previous[member]!r} and the current pack has {current[member]!r}"
            )
    return {
        "schema_version": COMPARISON_SCHEMA_VERSION,
        "engagement_type": current["engagement_type"],
        "period_end": current["period_end"],
        "previous": {
            "overall_status": previous["overall_status"],
            "acknowledgement": previous["acknowledgement"],
            "artefact_sha256": previous_artefacts,
        },
        "current": {
            "overall_status": current["overall_status"],
            "acknowledgement": current["acknowledgement"],
            "artefact_sha256": current_artefacts,
        },
        "scope_changes": _scope_changes(previous, current),
        "sources": _source_changes(previous, current),
        "findings": _finding_changes(previous, current),
        "comparison_boundary": COMPARISON_BOUNDARY,
    }
