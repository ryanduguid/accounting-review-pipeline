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


def calls(workflow):
    rows = []
    for route in (MODULE["COUNTS"] if workflow == "all" else [workflow]):
        for owner, count in MODULE["COUNTS"][route].items():
            rows.extend({"owner": owner, "kind": "workflow", "exit_code": 0, "expected_exit": 0, "failure": None}
                        for _ in range(count))
    projects = {r["owner"]: {"revision": "a" * 40} for r in rows}
    rows.extend({"owner": owner, "kind": "runtime", "exit_code": 0, "expected_exit": 0, "failure": None}
                for owner in projects)
    return rows, projects


@pytest.mark.parametrize("workflow", ["all", "quarter", "close-forecast", "job-cash", "grant-cash"])
def test_complete_selected_routes_validate_and_render(results, workflow):
    commands, projects = calls(workflow)
    lines = MODULE["validate"](results, workflow, commands, projects)
    MODULE["summary"](results, commands, projects, lines=lines)
    text = (results / "summary.md").read_text()
    assert "Fixture checks passed" in text and "does not approve" in text
    assert lines and "a" * 40 in text


@pytest.mark.parametrize("name,change", [
    ("quarter/quarter.json", lambda d: d.update(periods=[])),
    ("quarter/quarter.json", lambda d: d["periods"].pop()),
    ("quarter/quarter.json", lambda d: d["periods"][0].update(cutoff="../../elsewhere")),
    ("quarter/quarter.json", lambda d: d["periods"][1]["balances"].update(cash="22001")),
    ("quarter/2026-10-31/close/close-review-pack.json", lambda d: d.update(overall_status="PASS")),
    ("quarter/2026-11-30/comparison.json", lambda d: d.update(current_period="2026-12-31")),
    ("lumbridge-close/close-review-pack.json", lambda d: d.update(exceptions=[])),
    ("lumbridge-refresh/review.json", lambda d: d["actuals"].update(ending_cash="0")),
    ("job-base/project-cash.json", lambda d: d["weeks"].pop()),
    ("job-extra-cost/project-cash.json", lambda d: d["weeks"][-1].update(ending_cash="-434500")),
    ("job-base/project-cash.json", lambda d: d["weeks"][2].update(ending_cash="0")),
    ("job-extra-cost/project-cash.json", lambda d: d["weeks"][7].update(ending_cash="0")),
    ("grant-cash.json", lambda d: d["forecast"].update(closing_cash="34801")),
    ("grant-cash.json", lambda d: d.update(workpaper_status="RECONCILED")),
    ("grant-cash.json", lambda d: d["source_findings"].pop()),
    ("grant-cash.json", lambda d: d.update(unplanned_commitments=[])),
    ("grants/workpaper.json", lambda d: d.update(status="RECONCILED")),
])
def test_changed_outputs_are_rejected(results, name, change):
    path = results / name
    data = json.loads(path.read_text())
    change(data)
    path.write_text(json.dumps(data))
    with pytest.raises(ValueError):
        MODULE["validate"](results, "all", *calls("all"))


@pytest.mark.parametrize("kind", ["workflow", "runtime"])
def test_missing_command_is_rejected(results, kind):
    commands, projects = calls("all")
    commands.pop(next(i for i, c in enumerate(commands) if c["kind"] == kind))
    with pytest.raises(ValueError, match="coverage"):
        MODULE["validate"](results, "all", commands, projects)


def test_missing_output_is_rejected(results):
    (results / "quarter/2026-12-31/comparison.json").unlink()
    with pytest.raises(OSError):
        MODULE["validate"](results, "all", *calls("all"))


def test_failure_summary_cannot_claim_success_or_inject_markup(tmp_path):
    MODULE["summary"](tmp_path, [], {"close": {"revision": "<script>\n# injected"}},
                      failure="<script>\n# failed")
    text = (tmp_path / "summary.md").read_text()
    assert "Results are not verified" in text
    assert "<script>" not in text and "\n# injected" not in text and "Fixture checks passed" not in text
