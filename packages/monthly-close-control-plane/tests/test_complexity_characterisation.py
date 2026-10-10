"""Characterise validation order and diagnostics before helper extraction."""
import copy
import json
import runpy
import sys
from pathlib import Path

import pytest
from closecontrol.calculation_evidence import load
from closecontrol.cli import main
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError, SchemaError
from closecontrol.report import _as_json
from closecontrol.viewer import _verify_calculation_evidence, _verify_json_schema

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


@pytest.fixture
def document():
    return _as_json(review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        acknowledgement_path=EXAMPLES / "review_note.json",
    ))


@pytest.mark.parametrize("member,value,message", [
    ("overall_status", "APPROVED", "overall_status must be one of PASS, REVIEW, BLOCKED; got 'APPROVED'"),
    ("current_report_dates", [None], "current_report_dates must be a list of strings"),
    ("thresholds", {}, "thresholds must hold exactly absolute_variance, percentage_variance, reconciliation_tolerance"),
    ("source_sha256", {}, "source_sha256 must be a non-empty object"),
    ("exceptions", {}, "exceptions must be a list"),
    ("exceptions", [None], "exceptions[0] must be an object"),
    ("client_queries", {}, "client_queries must be a list"),
    ("client_queries", [None], "client_queries[0] must be an object"),
    ("acknowledgement", "reviewed", "acknowledgement must be null or an object"),
])
def test_schema_diagnostics_and_order(document, member, value, message):
    document[member] = value
    if member != "acknowledgement":
        document["acknowledgement"] = "also malformed"
    with pytest.raises(ControlInputError) as caught:
        _verify_json_schema(document)
    assert str(caught.value) == "close-review-pack.json: " + message


def test_threshold_and_source_errors_precede_exception_errors(document):
    document["exceptions"] = [None]
    document["thresholds"]["absolute_variance"] = "bad"
    with pytest.raises(ControlInputError) as caught:
        _verify_json_schema(document)
    assert str(caught.value) == "close-review-pack.json: thresholds.absolute_variance is not a decimal: 'bad'"
    document["thresholds"]["absolute_variance"] = "1000.00"
    document["source_sha256"] = {"current": "bad"}
    with pytest.raises(ControlInputError) as caught:
        _verify_json_schema(document)
    assert str(caught.value) == "close-review-pack.json: source_sha256['current'] is not a lowercase SHA-256 digest"


def test_query_id_validation_precedes_acknowledgement(document):
    document["client_queries"][0]["query_id"] = ""
    document["acknowledgement"] = "also malformed"
    with pytest.raises(ControlInputError) as caught:
        _verify_json_schema(document)
    assert str(caught.value) == "close-review-pack.json: client_queries[0].query_id must be a non-empty string"


def test_acknowledgement_field_type_precedes_effect_check(document):
    document["acknowledgement"]["comment"] = 1
    document["acknowledgement"]["effect"] = "approved"
    with pytest.raises(ControlInputError) as caught:
        _verify_json_schema(document)
    assert str(caught.value) == "close-review-pack.json: acknowledgement.comment must be a string"


@pytest.mark.parametrize("mutation,message", [
    ({"synthetic_input": "true"}, "synthetic_input must be a boolean."),
    ({"upstream": {"manifest": {"rate_table_uris": 1}}}, "manifest.rate_table_uris is int, not an array."),
    ({"normalised": {"values": {"levy": 1}}}, "normalised.values.levy is int, not a decimal string. A JSON number has already lost whatever the calculator meant by it."),
])
def test_evidence_malformed_fields_keep_their_first_error(tmp_path, mutation, message):
    record = json.loads((EXAMPLES / "calculation-evidence-coal-lsl-levy.json").read_text())
    record["calculation"].update(copy.deepcopy(mutation))
    record["calculation_sha256"] = "0" * 64
    path = tmp_path / "evidence.json"
    path.write_text(json.dumps(record), encoding="utf-8")
    with pytest.raises(SchemaError) as caught:
        load(path)
    assert str(caught.value) == f"{path}: {message}"


def test_evidence_unknown_members_precede_missing_members():
    with pytest.raises(ControlInputError) as caught:
        _verify_calculation_evidence({"unexpected": None}, "", {})
    assert str(caught.value) == "close-review-pack.json: calculation_evidence has unknown member(s): unexpected"


def test_evidence_required_type_precedes_effect_and_summary():
    with pytest.raises(ControlInputError) as caught:
        _verify_calculation_evidence({"supplied": [], "required": [1], "effect": "wrong"}, "", {})
    assert str(caught.value) == "close-review-pack.json: calculation_evidence.required must be a list of strings"


@pytest.mark.parametrize("value,message", [
    ("bad", "not a decimal: 'bad'"),
    ("-1", "must be a finite non-negative decimal"),
    ("NaN", "must be a finite non-negative decimal"),
])
def test_invalid_cli_money_is_a_usage_error(tmp_path, capsys, value, message):
    assert main(["review", "--current", str(EXAMPLES / "current_trial_balance.csv"),
                 "--prior", str(EXAMPLES / "prior_trial_balance.csv"),
                 "--output", str(tmp_path / "pack"), "--absolute-threshold", value]) == 1
    assert message in capsys.readouterr().err
    assert not (tmp_path / "pack").exists()


@pytest.mark.parametrize("command", ["review", "workbench"])
def test_quiet_pass_prints_the_existing_banner(tmp_path, capsys, command):
    header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n"
    paths = []
    for month in (7, 8):
        path = tmp_path / f"tb-{month}.csv"
        path.write_text(header + f"2026-{month:02}-31,Synthetic,Assets,A,Bank,090,100,0,100,0\n"
                        + f"2026-{month:02}-31,Synthetic,Equity,E,Capital,300,0,100,0,100\n",
                        encoding="utf-8")
        paths.append(path)
    assert main([command, "--current", str(paths[1]), "--prior", str(paths[0]),
                 "--output", str(tmp_path / "pack")]) == 0
    prefix = "close-control workbench" if command == "workbench" else "close-control"
    assert f"{prefix}: PASS; 0 exception(s); 0 client query(ies) drafted\n" in capsys.readouterr().out


@pytest.mark.parametrize("prior_date,reset", [("2026-06-30", True), ("2026-07-30", False)])
def test_driver_cli_explains_the_financial_year_reset(tmp_path, capsys, prior_date, reset):
    prior = tmp_path / "prior.csv"
    source = (EXAMPLES / "prior_trial_balance.csv").read_text()
    original_date = source.splitlines()[1].split(",")[0]
    prior.write_text(source.replace(original_date, prior_date), encoding="utf-8")
    pack = tmp_path / "pack"
    assert main(["review", "--current", str(EXAMPLES / "current_trial_balance.csv"),
                 "--prior", str(prior), "--output", str(pack)]) == 2
    capsys.readouterr()
    assert main(["drivers", "--pack-dir", str(pack), "--transactions",
                 str(EXAMPLES / "variance_transactions.csv"), "--currency", "AUD",
                 "--output", str(tmp_path / "drivers")]) == 2
    message = "The pack crosses a 30 June reset; profit-and-loss movements are not comparable."
    assert (message in capsys.readouterr().out) is reset


@pytest.mark.filterwarnings("ignore:.*found in sys.modules.*:RuntimeWarning")
def test_module_entrypoint_preserves_help_exit(monkeypatch, capsys):
    monkeypatch.setattr(sys, "argv", ["close-control", "--help"])
    with pytest.raises(SystemExit) as caught:
        runpy.run_module("closecontrol.cli", run_name="__main__")
    assert caught.value.code == 0
    assert "Create a review-first monthly close control pack" in capsys.readouterr().out
