"""Comparison scope changes and the gates on packs, periods and responses."""
from __future__ import annotations

import dataclasses
import json

import pytest
from closecontrol.comparison import _scope_changes, _verified, compare_packs
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.report import write_review_pack

from test_comparison import _pair


def _run(previous=None, **changes):
    """Return a minimal verified pack document, the shape _scope_changes reads."""
    document = {"controls_not_run": [], "thresholds": {"absolute": "1000"}, "source_sha256": {},
                "current_report_dates": ["2026-09-30"], "prior_report_dates": ["2026-08-31"],
                "exceptions": [], "overall_status": "PASS"}
    if previous:
        document.update(current_report_dates=["2026-08-31"], prior_report_dates=["2026-07-31"])
    document.update(changes)
    return document


DROP = object()


def _equity(currency="AUD", accounts=("E",)):
    return {"currency": currency, "accounts": [{"account_id": key} for key in accounts]}


def test_contiguous_runs_with_the_same_scope_record_no_change():
    assert _scope_changes(_run(previous=True), _run()) == []  # nosec B101: pytest outcome assertion


def test_rerun_of_the_same_period_is_not_a_gap():
    assert _scope_changes(_run(), _run()) == []  # nosec B101: pytest outcome assertion


@pytest.mark.parametrize("previous,current,expected", [
    ({"controls_not_run": DROP}, {}, ["controls_not_run", "control_coverage_unknown"]),
    ({"source_sha256": {"mapping_policy": "a" * 64}}, {}, ["mapping_policy"]),
    ({"calculation_evidence": {"required": ["fbt"]}}, {}, ["required_calculations"]),
    ({"equity_reconciliation": _equity()}, {}, ["equity_control_coverage"]),
    ({"equity_reconciliation": _equity()}, {"equity_reconciliation": _equity()}, []),
    ({"equity_reconciliation": _equity()}, {"equity_reconciliation": _equity("NZD")}, ["equity_account_scope"]),
    ({"equity_reconciliation": _equity()}, {"equity_reconciliation": _equity(accounts=("E", "F"))},
     ["equity_account_scope"]),
    ({"current_report_dates": ["2026-07-31"]}, {}, ["periods_not_contiguous"]),
    ({}, {"exceptions": [{"control": "financial_year_reset"}]}, ["financial_year_reset"]),
    ({"overall_status": "BLOCKED"}, {}, ["blocked_run"]),
])
def test_scope_change_is_named(previous, current, expected):
    old = _run(previous=True)
    old.update(previous)
    old = {key: value for key, value in old.items() if value is not DROP}
    assert _scope_changes(old, _run(**current)) == expected  # nosec B101: pytest outcome assertion


def _rewrite(tmp_path, inputs, change):
    """Rewrite the current pack from a changed ClosePack and return its directory."""
    pack = review_close(current_path=inputs["current_tb"], prior_path=inputs["previous_tb"],
                        subledger_path=tmp_path / "subledger.csv")
    output = tmp_path / "rewritten"
    write_review_pack(change(pack), output)
    return output


def test_pack_date_that_disagrees_with_its_trial_balance_is_refused(tmp_path):
    inputs = _pair(tmp_path)
    output = _rewrite(tmp_path, inputs,
                      lambda pack: dataclasses.replace(pack, current_report_dates=("2026-09-29",)))
    with pytest.raises(ControlInputError, match="Comparison trial-balance date does not match the pack"):
        _verified(output, inputs["current_tb"])


def test_pack_finding_for_another_tenant_is_refused(tmp_path):
    inputs = _pair(tmp_path)

    def other_tenant(pack):
        exceptions = tuple(dataclasses.replace(item, tenant="Another entity") for item in pack.exceptions)
        return dataclasses.replace(pack, exceptions=exceptions)

    output = _rewrite(tmp_path, inputs, other_tenant)
    with pytest.raises(ControlInputError, match="Pack finding names another tenant"):
        _verified(output, inputs["current_tb"])


def test_current_pack_cannot_precede_the_previous_pack(tmp_path):
    inputs = _pair(tmp_path)
    with pytest.raises(ControlInputError, match="Current pack cannot precede the previous pack"):
        compare_packs(previous_pack=inputs["current_pack"], previous_tb=inputs["current_tb"],
                      current_pack=inputs["previous_pack"], current_tb=inputs["previous_tb"])


def test_changed_account_population_is_a_scope_change(tmp_path):
    inputs = _pair(tmp_path)
    current = inputs["current_tb"]
    current.write_text(current.read_text(encoding="utf-8")
                       + "2026-09-30,Synthetic,Expenses,X,Sundry,400,0,0,0,0\n", encoding="utf-8")
    replacement = tmp_path / "replacement"
    write_review_pack(review_close(current_path=current, prior_path=inputs["previous_tb"],
                                   subledger_path=tmp_path / "subledger.csv"), replacement)
    inputs["current_pack"] = replacement
    result = compare_packs(**inputs)
    assert result["scope_changes"] == ["trial_balance_account_population"]  # nosec B101: pytest outcome assertion
    new = [item for item in result["findings"] if item["account_id"] == "X"]
    assert [item["change"] for item in new] == ["NEW"]  # nosec B101: pytest outcome assertion


def test_response_date_must_parse(tmp_path):
    inputs = _pair(tmp_path)
    base = compare_packs(**inputs)
    document = {"schema_version": 1,
                "pack_sha256": base["current_artefact_sha256"]["close-review-pack.json"],
                "responses": [{"query_id": base["queries"][0]["query_id"], "explanation": "Check",
                               "evidence_reference": "Item", "reviewer": "AB",
                               "reviewed_on": "first of October"}]}
    path = tmp_path / "response.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ControlInputError, match="Response reviewed_on must be an ISO date"):
        compare_packs(**inputs, responses=path)
