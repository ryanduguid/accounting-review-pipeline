"""Acceptance checks for the fixed fabricated joined examples, not client data."""
from __future__ import annotations

import json
import re
from collections import Counter
from decimal import Decimal
from pathlib import Path

CUTOFFS = ("2026-09-30", "2026-10-31", "2026-11-30", "2026-12-31")
CASH = ("20000", "22000", "24500", "29500")
COUNTS = {"close-forecast": {"fpa": 2, "close": 2}, "quarter": {"fpa": 1, "close": 8},
          "job-cash": {"wip": 2, "fpa": 2}, "grant-cash": {"grants": 1, "fpa": 1}}
GRANT_FINDINGS = {("MISSING_SOURCE_EVIDENCE", "L3"), ("OUTSIDE_AGREEMENT_PERIOD", "A3"),
                  ("MISSING_ALLOCATION_EVIDENCE", "A4"), ("UNAPPROVED_ALLOCATION", "A4"),
                  ("UNALLOCATED_SOURCE", "L4")}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(root, name):
    return json.loads((root / name).read_text(encoding="utf-8"), parse_float=Decimal)


def quarter_periods(periods):
    """Check dates before using any producer-supplied value in an output path."""
    require(isinstance(periods, list) and len(periods) == 4, "Quarter needs opening balances and three monthly periods")
    require([p["cutoff"] for p in periods] == list(CUTOFFS), "Quarter cutoffs differ from the fabricated fixture")
    require([Decimal(p["balances"]["cash"]) for p in periods] == list(map(Decimal, CASH)),
            "Quarter cash balances differ from the fabricated fixture")


def validate(output: Path, workflow: str, calls: list, projects: dict) -> list[str]:
    """Return display lines only after every selected route passes its contract."""
    routes = list(COUNTS) if workflow == "all" else [workflow]
    expected = Counter()
    for route in routes:
        expected.update(COUNTS[route])
    require(Counter(c["owner"] for c in calls if c["kind"] == "workflow") == expected,
            "Workflow command coverage is incomplete")
    require(Counter(c["owner"] for c in calls if c["kind"] == "runtime") == Counter({p: 1 for p in projects}),
            "Runtime probe coverage is incomplete")
    require(all(c["kind"] in {"workflow", "runtime"} and c["failure"] is None and
                c["exit_code"] == c["expected_exit"] for c in calls), "A command did not complete as expected")
    lines = []

    def close(name, label):
        pack = read(output, name)
        require(pack["overall_status"] == "REVIEW" and bool(pack["exceptions"]),
                f"{label} must retain REVIEW findings")
        count = len(pack["exceptions"])
        return f"{label}: REVIEW ({count} {'finding' if count == 1 else 'findings'})."

    if "close-forecast" in routes:
        lines.append(close("lumbridge-close/close-review-pack.json", "Close"))
        refresh = read(output, "lumbridge-refresh/review.json")
        require(refresh["closed_through"] == "2026-10-31" and
                Decimal(refresh["actuals"]["ending_cash"]) == Decimal("-1960"), "Cash refresh differs from the fixture")
        lines.append("Cash refresh: AUD -1,960.")
    if "quarter" in routes:
        quarter_periods(read(output, "quarter/quarter.json")["periods"])
        for index, cutoff in enumerate(CUTOFFS[1:], 1):
            lines.append(close(f"quarter/{cutoff}/close/close-review-pack.json", f"Quarter {cutoff}"))
            if index > 1:
                comparison = read(output, f"quarter/{cutoff}/comparison.json")
                require((comparison["previous_period"], comparison["current_period"]) == (CUTOFFS[index - 1], cutoff),
                        "Quarter comparison periods differ")
                require(comparison["previous_status"] == comparison["current_status"] == "REVIEW",
                        "Quarter comparisons must retain REVIEW")
        lines.append("Quarter cash: AUD 20,000 / 22,000 / 24,500 / 29,500.")
    if "job-cash" in routes:
        for name, expected_cash in (("job-base", "-434500"), ("job-extra-cost", "-544500")):
            weeks = read(output, f"{name}/project-cash.json")["weeks"]
            require([w["week"] for w in weeks] == list(range(1, 14)), "WIP cash needs thirteen ordered weeks")
            require(Decimal(weeks[-1]["ending_cash"]) == Decimal(expected_cash), "WIP cash differs from the fixture")
        lines.append("Project cash at week 13: AUD -434,500 baseline; AUD -544,500 with extra completion costs.")
    if "grant-cash" in routes:
        cash = read(output, "grant-cash.json")
        workpaper = read(output, "grants/workpaper.json")
        require(cash["workpaper_status"] == workpaper["status"] == "REVIEW", "Grant REVIEW status was lost")
        for findings in (cash["source_findings"], workpaper["findings"]):
            require(len(findings) == len(GRANT_FINDINGS) and
                    {(f["code"], f["reference"]) for f in findings} == GRANT_FINDINGS, "Grant source findings changed")
        require(Decimal(cash["forecast"]["closing_cash"]) == Decimal("34800") and
                cash["liquidity"]["status"] == "NO_SHORTFALL", "Grant cash differs from the fixture")
        require(Decimal(cash["known_unpaid_allocations"]) == Decimal("3200") and
                Decimal(cash["planned_commitment_cash"]) == Decimal("200") and
                len(cash["unplanned_commitments"]) == 1 and
                cash["unplanned_commitments"][0]["allocation_id"] == "A4" and
                Decimal(cash["unplanned_commitments"][0]["amount"]) == Decimal("3000"), "Grant unresolved commitments changed")
        lines.append("Grant closing cash: AUD 34,800; NO_SHORTFALL. REVIEW remains with five source findings and AUD 3,000 unplanned commitments.")
    return lines


def summary(output: Path, calls: list, projects: dict, lines: list[str] | None = None, failure: str | None = None):
    """Write bounded, escaped summary text; never include subprocess diagnostics."""
    def safe(value):
        return re.sub(r"[^a-zA-Z0-9 .,;:_()/=-]", "?", str(value))[:240]

    complete = lines is not None and failure is None
    text = ["# Joined accounting examples", "", "Fixture checks passed." if complete else "Run failed. Results are not verified.", "",
            f"Completed commands: {sum(c['failure'] is None and c['exit_code'] == c['expected_exit'] for c in calls)} of {len(calls)} attempted.", ""]
    if failure:
        text.extend([f"Failure: {safe(failure)}", ""])
    text.extend(f"- {safe(line)}" for line in (lines or []))
    text.extend(["", "Source revisions:", ""])
    text.extend(f"- {safe(owner)}: {safe(p.get('revision') or 'uncommitted')}" for owner, p in sorted(projects.items()))
    text.extend(["", "Fabricated examples only. Passing integration does not approve a close, grant acquittal or payment.", ""])
    (output / "summary.md").write_text("\n".join(text), encoding="utf-8")
