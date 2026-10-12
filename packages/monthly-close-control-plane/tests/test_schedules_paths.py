"""Schedule input gates that refuse malformed records before any arithmetic."""
from __future__ import annotations

import json
from pathlib import Path

import pytest
from closecontrol.errors import ControlInputError
from closecontrol.schedules import review_schedule

EXAMPLES = Path(__file__).resolve().parents[1] / "examples" / "schedules"


def example(kind):
    return json.loads((EXAMPLES / f"{kind}.json").read_text(encoding="utf-8"))


def run(tmp_path, kind, data):
    source = tmp_path / "input.json"
    source.write_text(json.dumps(data), encoding="utf-8")
    return review_schedule(source, kind=kind)


def test_unknown_kind_is_refused(tmp_path):
    with pytest.raises(ControlInputError, match="Unknown schedule kind"):
        run(tmp_path, "payroll", example("expenses"))


@pytest.mark.parametrize("cutoff", [None, "30/09/2026"])
def test_cutoff_must_be_an_iso_date(tmp_path, cutoff):
    data = example("expenses")
    data["cutoff"] = cutoff
    with pytest.raises(ControlInputError, match="cutoff: expected YYYY-MM-DD"):
        run(tmp_path, "expenses", data)


@pytest.mark.parametrize("tenant", ["", " Padded", "Line\nbreak", 7])
def test_tenant_must_be_plain_text(tmp_path, tenant):
    data = example("expenses")
    data["tenant"] = tenant
    with pytest.raises(ControlInputError, match="tenant: expected non-blank text"):
        run(tmp_path, "expenses", data)


def test_population_must_be_an_array(tmp_path):
    data = example("expenses")
    data["invoices"] = {"I1": data["invoices"][0]}
    with pytest.raises(ControlInputError, match="invoices: expected an array"):
        run(tmp_path, "expenses", data)


def test_invoice_amount_must_be_positive(tmp_path):
    data = example("expenses")
    data["invoices"][0]["amount"] = "0.00"
    with pytest.raises(ControlInputError, match="Invoice amount must be positive"):
        run(tmp_path, "expenses", data)


def test_invoice_evidence_must_be_text(tmp_path):
    data = example("expenses")
    data["invoices"][0]["evidence"] = None
    with pytest.raises(ControlInputError, match="evidence must be text"):
        run(tmp_path, "expenses", data)


@pytest.mark.parametrize("field,value", [("amount", "-1.00"), ("approved", "yes")])
def test_claim_needs_positive_amount_and_boolean_approval(tmp_path, field, value):
    data = example("expenses")
    data["claims"][0][field] = value
    with pytest.raises(ControlInputError, match="Claims require a positive amount and boolean approved"):
        run(tmp_path, "expenses", data)


def test_claim_against_unknown_invoice_is_a_finding(tmp_path):
    data = example("expenses")
    data["claims"][0]["invoice_id"] = "I9"
    result = run(tmp_path, "expenses", data)
    assert {"code": "UNKNOWN_INVOICE", "id": "C1"} in result["findings"]  # nosec B101: pytest outcome assertion
    assert result["status"] == "REVIEW"  # nosec B101: pytest outcome assertion


@pytest.mark.parametrize("field,value", [("amount", "0.00"), ("date", "2026-10-01")])
def test_payment_needs_positive_amount_on_or_before_cutoff(tmp_path, field, value):
    data = example("expenses")
    data["payments"][0][field] = value
    with pytest.raises(ControlInputError, match="Payments require a positive amount and date on or before cutoff"):
        run(tmp_path, "expenses", data)


@pytest.mark.parametrize("weight", [1, "1.5", "0.1234567"])
def test_mapping_weight_must_be_a_decimal_string_up_to_one(tmp_path, weight):
    data = example("migration")
    data["mapping"][0]["weight"] = weight
    with pytest.raises(ControlInputError, match=r"Mapping weight must be a decimal string in \(0, 1\]"):
        run(tmp_path, "migration", data)


def test_zero_mapping_weight_is_refused(tmp_path):
    data = example("migration")
    data["mapping"][0]["weight"] = "0.0"
    with pytest.raises(ControlInputError, match="Mapping weights must be positive"):
        run(tmp_path, "migration", data)


def test_repeated_source_target_pair_is_refused(tmp_path):
    data = example("migration")
    data["mapping"][0]["weight"] = "0.5"
    data["mapping"].append({**data["mapping"][0], "mapping_id": "M2"})
    with pytest.raises(ControlInputError, match="source/target pairs unique"):
        run(tmp_path, "migration", data)


def test_mapping_weights_for_a_source_must_sum_to_one(tmp_path):
    data = example("migration")
    data["mapping"][0]["weight"] = "0.5"
    with pytest.raises(ControlInputError, match="must sum to 1"):
        run(tmp_path, "migration", data)


@pytest.mark.parametrize("entities", ["A,B", ["A"]])
def test_interentity_needs_two_declared_entities(tmp_path, entities):
    data = example("interentity")
    data["entities"] = entities
    with pytest.raises(ControlInputError, match="Supply at least 2 explicit entity IDs"):
        run(tmp_path, "interentity", data)


def test_interentity_entities_must_be_unique(tmp_path):
    data = example("interentity")
    data["entities"] = ["A", "A"]
    with pytest.raises(ControlInputError, match="Entity IDs must be unique"):
        run(tmp_path, "interentity", data)


@pytest.mark.parametrize("field,value", [("entity", "C"), ("counterparty", "C"), ("counterparty", "A")])
def test_interentity_record_needs_two_distinct_declared_entities(tmp_path, field, value):
    data = example("interentity")
    data["records"][0][field] = value
    with pytest.raises(ControlInputError, match="Each record requires 2 distinct declared entity IDs"):
        run(tmp_path, "interentity", data)


@pytest.mark.parametrize("field,value", [("date", "2026-10-01"), ("disputed", "no")])
def test_interentity_record_needs_cutoff_date_and_boolean_dispute(tmp_path, field, value):
    data = example("interentity")
    data["records"][0][field] = value
    with pytest.raises(ControlInputError, match="Records require a date on or before cutoff and boolean disputed"):
        run(tmp_path, "interentity", data)
