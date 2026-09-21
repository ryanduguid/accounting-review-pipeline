from __future__ import annotations

import copy
import json
from decimal import Decimal, localcontext
from pathlib import Path

import pytest
from closecontrol.cli import main
from closecontrol.errors import ControlInputError
from closecontrol.schedules import review_schedule

EXAMPLES = Path(__file__).resolve().parents[1] / "examples" / "schedules"


def example(kind):
    return json.loads((EXAMPLES / f"{kind}.json").read_text())


def run(tmp_path, kind, data):
    source = tmp_path / "input.json"
    source.write_text(json.dumps(data), encoding="utf-8")
    return review_schedule(source, kind=kind)


def test_reimbursement_overpayment_retains_both_records(tmp_path):
    result = run(tmp_path, "expenses", example("expenses"))
    assert result["status"] == "REVIEW"
    assert result["claims"][0]["remaining"] == "-20.00"
    assert len(result["payments"]) == 2
    assert result["ledger_difference"] == "0.00"
    assert {row["code"] for row in result["findings"]} == {"OVERPAID"}


@pytest.mark.parametrize("kind,field", [("expenses", "invoices"), ("migration", "before"), ("interentity", "records")])
def test_empty_population_does_not_establish_agreement(tmp_path, kind, field):
    data = example(kind)
    data[field] = []
    with pytest.raises(ControlInputError):
        run(tmp_path, kind, data)


def test_expense_flags_are_independent(tmp_path):
    data = example("expenses")
    data["invoices"][0]["evidence"] = ""
    data["invoices"].append({**data["invoices"][0], "invoice_id": "I2"})
    data["claims"].append({**data["claims"][0], "claim_id": "C2", "approved": False})
    data["payments"][1]["claim_id"] = "missing"
    result = run(tmp_path, "expenses", data)
    assert {row["code"] for row in result["findings"]} >= {
        "DUPLICATE_INVOICE_REFERENCE", "MISSING_RECEIPT", "OVER_ALLOCATED",
        "UNAPPROVED_CLAIM", "UNLINKED_PAYMENT", "UNPAID_APPROVED", "CLEARING_DIFFERENCE",
    }


def test_paid_claim_passes(tmp_path):
    data = example("expenses")
    data["payments"] = [{**data["payments"][0], "amount": "100.00"}]
    data["ledger_balance"] = "0"
    assert run(tmp_path, "expenses", data)["status"] == "PASS"


def test_equal_totals_do_not_hide_missing_and_duplicate_invoices(tmp_path):
    result = run(tmp_path, "migration", example("migration"))
    assert result["before_total"] == result["after_total"] == "200.00"
    assert len(result["differences"]) == 2
    assert {row["item_id"] for row in result["differences"]} == {"I1", "I2"}
    assert any(row["code"] == "DUPLICATE_ITEM" for row in result["findings"])


def test_explicit_split_account_mapping(tmp_path):
    data = example("migration")
    data["before"] = data["before"][:1]
    data["mapping"][0]["weight"] = "0.4"
    data["mapping"].append({**data["mapping"][0], "mapping_id": "M2", "target_account": "OTHER", "weight": "0.6"})
    data["after"] = [{**data["after"][0], "amount": "40"}, {**data["after"][0], "row_id": "split", "account": "OTHER", "amount": "60"}]
    assert run(tmp_path, "migration", data)["status"] == "PASS"


@pytest.mark.parametrize("field,new", [("reference", "lost"), ("tax_code", "OTHER"), ("item_id", "lost")])
def test_migration_identifies_lost_metadata(tmp_path, field, new):
    data = example("migration")
    data["before"] = data["before"][:1]
    data["after"] = data["after"][:1]
    data["after"][0][field] = new
    assert len(run(tmp_path, "migration", data)["differences"]) == 2


def test_unmapped_accounts_are_visible(tmp_path):
    data = example("migration")
    data["mapping"] = []
    assert any(row["code"] == "UNMAPPED_ACCOUNT" for row in run(tmp_path, "migration", data)["findings"])


def test_reciprocal_difference_is_not_an_elimination(tmp_path):
    result = run(tmp_path, "interentity", example("interentity"))
    assert result["pairs"][0]["difference"] == "3000"
    assert result["pairs"][0]["records"]["B"][0]["amount"] == "-47000"
    assert "elimination" not in result


def test_offsetting_unmatched_references_remain_unmatched(tmp_path):
    data = example("interentity")
    data["records"][1]["amount"] = "-50000"
    data["records"][1]["reference"] = "OTHER"
    result = run(tmp_path, "interentity", data)
    assert len(result["pairs"]) == 2
    assert all(row["status"] == "UNMATCHED" for row in result["pairs"])


def test_disputed_but_balanced_pair_needs_review(tmp_path):
    data = example("interentity")
    data["records"][1]["amount"] = "-50000"
    data["records"][1]["disputed"] = True
    result = run(tmp_path, "interentity", data)
    assert result["status"] == "REVIEW"
    assert result["pairs"][0]["status"] == "AGREES"


@pytest.mark.parametrize("kind", ["expenses", "migration", "interentity"])
@pytest.mark.parametrize("field,value", [("schema_version", True), ("cutoff", "20260930"), ("currency", "aud"), ("extra", 1)])
def test_strict_outer_contract(tmp_path, kind, field, value):
    data = example(kind)
    data[field] = value
    with pytest.raises(ControlInputError):
        run(tmp_path, kind, data)


@pytest.mark.parametrize("amount", ["NaN", "Infinity", "1e3", "", "0.001", 10, None])
def test_malformed_amounts_fail(tmp_path, amount):
    data = example("expenses")
    data["claims"][0]["amount"] = amount
    with pytest.raises(ControlInputError):
        run(tmp_path, "expenses", data)


@pytest.mark.parametrize("kind,collection", [("expenses", "claims"), ("migration", "before"), ("interentity", "records")])
def test_duplicate_source_identifiers_fail(tmp_path, kind, collection):
    data = example(kind)
    data[collection].append(copy.deepcopy(data[collection][0]))
    with pytest.raises(ControlInputError, match="duplicate"):
        run(tmp_path, kind, data)


def test_precision_independent_of_caller(tmp_path):
    with localcontext() as context:
        context.prec = 2
        result = run(tmp_path, "interentity", example("interentity"))
    assert Decimal(result["pairs"][0]["difference"]) == 3000


def test_cli_and_duplicate_json(tmp_path, capsys):
    assert main(["schedule", "--kind", "interentity", "--input", str(EXAMPLES / "interentity.json")]) == 2
    assert json.loads(capsys.readouterr().out)["pairs"][0]["difference"] == "3000"
    source = tmp_path / "duplicate.json"
    source.write_text('{"schema_version":1,"schema_version":1}')
    assert main(["schedule", "--kind", "expenses", "--input", str(source)]) == 1
