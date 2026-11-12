# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""Reject incomplete fabricated workflows even when their commands exit normally."""
import json
import runpy
from pathlib import Path

import pytest

MODULE = runpy.run_path(str(Path(__file__).parents[1] / "examples/utility_results.py"))


@pytest.fixture
def results(tmp_path):
    def save(name, value):
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding="utf-8")

    save("lumbridge-close/close-review-pack.json", {"overall_status": "REVIEW", "exceptions": [{}]})
    save("lumbridge-refresh/review.json", {"closed_through": "2026-10-31", "actuals": {"ending_cash": "-1960"}})
    save("quarter/quarter.json", {"periods": [{"cutoff": date, "balances": {"cash": cash}}
         for date, cash in zip(MODULE["CUTOFFS"], MODULE["CASH"])]})
    for index, date in enumerate(MODULE["CUTOFFS"][1:], 1):
        save(f"quarter/{date}/close/close-review-pack.json", {"overall_status": "REVIEW", "exceptions": [{}]})
        if index > 1:
            save(f"quarter/{date}/comparison.json", {"previous_period": MODULE["CUTOFFS"][index - 1],
                 "current_period": date, "previous_status": "REVIEW", "current_status": "REVIEW"})
    for label, cash in (("job-base", "-434500"), ("job-extra-cost", "-544500")):
        save(f"{label}/project-cash.json", {"weeks": [{"week": i, "ending_cash": value} for i, value in enumerate(["-140000"] * 4 + ["5500"] * 6 + [cash] * 3, 1)]})
    findings = [{"code": code, "reference": ref} for code, ref in sorted(MODULE["GRANT_FINDINGS"])]
    save("grants/workpaper.json", {"status": "REVIEW", "findings": findings})
    save("grant-cash.json", {"workpaper_status": "REVIEW", "source_findings": findings,
         "forecast": {"closing_cash": "34800"}, "liquidity": {"status": "NO_SHORTFALL"},
         "known_unpaid_allocations": "3200", "planned_commitment_cash": "200",
         "unplanned_commitments": [{"allocation_id": "A4", "amount": "3000"}]})
    return tmp_path
