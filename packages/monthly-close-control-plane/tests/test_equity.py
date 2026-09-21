"""Independent equity arithmetic, evidence gates and all pack projections."""
import hashlib
import json
from decimal import Decimal, localcontext

import pytest
from closecontrol.cli import main
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.report import write_review_pack
from closecontrol.viewer import verify_pack

HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"


def _tb(path, when, balances, section="Equity"):
    rows = []
    total = sum((Decimal(value) for value in balances.values()), Decimal(0))
    for account, value in {**balances, "A": -total}.items():
        debit, credit = max(-Decimal(value), 0), max(Decimal(value), 0)
        rows.append(f"{when},Synthetic,{section if account != 'A' else 'Assets'},"
                    f"{account},Account {account},{account},{debit},{credit},{debit},{credit}")
    path.write_text(HEADER + "\n" + "\n".join(rows) + "\n", encoding="utf-8")


def _inputs(tmp_path, *, opening=None, closing=None, movements=None, section="Equity"):
    opening = opening or {"E": "100000"}
    closing = closing or {"E": "129750"}
    prior, current = tmp_path / "prior.csv", tmp_path / "current.csv"
    _tb(prior, "2026-08-31", opening)
    _tb(current, "2026-09-30", closing, section)
    if movements is None:
        movements = [
            {"movement_id": "capital", "date": "2026-09-01", "account_id": "E",
             "debit": "0", "credit": "25000", "description": "Contribution",
             "evidence_reference": "Synthetic contribution record"},
            {"movement_id": "withdrawal", "date": "2026-09-10", "account_id": "E",
             "debit": "10000", "credit": "0", "description": "Withdrawal",
             "evidence_reference": "Synthetic withdrawal record"},
            {"movement_id": "profit", "date": "2026-09-30", "account_id": "E",
             "debit": "0", "credit": "15000", "description": "Evidenced profit transfer",
             "evidence_reference": "Synthetic closing transfer"},
        ]
    document = {
        "schema_version": 1, "tenant": "Synthetic", "opening_date": "2026-08-31",
        "closing_date": "2026-09-30", "currency": "AUD", "basis": "ledger_equity",
        "prior_source_sha256": hashlib.sha256(prior.read_bytes()).hexdigest(),
        "current_source_sha256": hashlib.sha256(current.read_bytes()).hexdigest(),
        "equity_account_ids": list(opening), "complete": True, "movements": movements,
    }
    schedule = tmp_path / "equity.json"
    schedule.write_text(json.dumps(document), encoding="utf-8")
    return dict(prior_path=prior, current_path=current, equity_schedule_path=schedule,
                equity_currency="AUD", equity_currency_evidence="Both source reports, metadata",
                absolute_threshold=Decimal("999999999")), document


def _change(inputs, document):
    inputs["equity_schedule_path"].write_text(json.dumps(document), encoding="utf-8")


def test_worked_example_has_250_difference_and_visible_evidence(tmp_path):
    inputs, _ = _inputs(tmp_path)
    pack = review_close(**inputs)
    assert pack.status == "REVIEW"
    result = pack.equity_reconciliation
    assert result["total"] == {"opening": "100000", "movement": "30000",
                               "expected_close": "130000", "actual_close": "129750",
                               "unexplained": "-250"}
    assert len(pack.client_queries) == 1
    output = tmp_path / "pack"
    write_review_pack(pack, output)
    document, summary, exceptions, queries, _ = verify_pack(output)
    assert document["equity_reconciliation"] == result
    assert "130000" in summary and "Synthetic closing transfer" in summary
    assert exceptions[0]["difference"] == "-250.00"
    assert queries[0]["control"] == "equity_reconciliation"


def test_matching_schedule_and_explicit_nil_movement(tmp_path):
    inputs, _ = _inputs(tmp_path, opening={"E": "100"}, closing={"E": "100"}, movements=[])
    pack = review_close(**inputs)
    assert pack.status == "PASS"
    assert pack.equity_reconciliation["total"]["movement"] == "0"
    write_review_pack(pack, tmp_path / "pack")
    assert verify_pack(tmp_path / "pack")[0]["equity_reconciliation"]["status"] == "PASS"


def test_debit_equity_and_loss_transfer_are_not_inferred_from_profit(tmp_path):
    movement = {"movement_id": "loss", "date": "2026-09-30", "account_id": "E",
                "debit": "25", "credit": "0", "description": "Loss transfer",
                "evidence_reference": "Independent closing record"}
    inputs, _ = _inputs(tmp_path, opening={"E": "-100"}, closing={"E": "-125"},
                         movements=[movement])
    pack = review_close(**inputs)
    assert pack.status == "PASS"
    assert pack.equity_reconciliation["total"]["expected_close"] == "-125"


@pytest.mark.parametrize("difference,expected", [
    ("0.001", "PASS"), ("-0.001", "PASS"), ("0.0011", "REVIEW"), ("-0.0011", "REVIEW")])
def test_subcent_tolerance_is_inclusive_and_preserved(tmp_path, difference, expected):
    inputs, _ = _inputs(tmp_path, opening={"E": "100"},
                         closing={"E": str(Decimal(100) + Decimal(difference))}, movements=[])
    pack = review_close(**inputs, reconciliation_tolerance=Decimal("0.001"))
    assert pack.status == expected
    assert Decimal(pack.equity_reconciliation["total"]["unexplained"]) == Decimal(difference)
    write_review_pack(pack, tmp_path / "pack")
    verify_pack(tmp_path / "pack")


def test_equal_opposite_errors_do_not_cancel(tmp_path):
    inputs, _ = _inputs(tmp_path, opening={"E": "100", "F": "100"},
                         closing={"E": "101", "F": "99"}, movements=[])
    pack = review_close(**inputs)
    assert pack.status == "REVIEW"
    assert len(pack.exceptions) == 2
    assert pack.equity_reconciliation["total"]["unexplained"] == "0"


def test_individually_small_errors_can_exceed_aggregate_tolerance(tmp_path):
    inputs, _ = _inputs(tmp_path, opening={"E": "100", "F": "100"},
                         closing={"E": "100.009", "F": "100.009"}, movements=[])
    pack = review_close(**inputs)
    assert pack.status == "REVIEW"
    assert len(pack.exceptions) == 1
    assert pack.exceptions[0].difference == Decimal(".018")
    assert pack.client_queries == ()


@pytest.mark.parametrize("field,value", [
    ("complete", False), ("tenant", "Other"), ("currency", "USD"),
    ("prior_source_sha256", "0" * 64), ("current_source_sha256", "0" * 64),
    ("opening_date", "2026-08-30"), ("closing_date", "2026-10-01"),
])
def test_semantic_evidence_failure_blocks_without_substitute_numbers(tmp_path, field, value):
    inputs, document = _inputs(tmp_path)
    document[field] = value
    _change(inputs, document)
    pack = review_close(**inputs)
    assert pack.status == "BLOCKED"
    assert pack.equity_reconciliation["accounts"] == []
    assert pack.equity_reconciliation["total"] is None
    write_review_pack(pack, tmp_path / "pack")
    verify_pack(tmp_path / "pack")


@pytest.mark.parametrize("field", ["equity_currency", "equity_currency_evidence"])
def test_missing_independent_currency_confirmation_blocks(tmp_path, field):
    inputs, _ = _inputs(tmp_path, closing={"E": "130000"})
    inputs[field] = None
    assert review_close(**inputs).status == "BLOCKED"


@pytest.mark.parametrize("change", [
    "unknown", "duplicate_account", "duplicate_movement", "both_sides", "zero_sides",
    "nan", "numeric_amount", "outside_date", "unknown_account", "empty_evidence",
    "schema_bool", "complete_string", "invalid_date", "extra_movement",
])
def test_malformed_schedule_is_an_input_error(tmp_path, change):
    inputs, document = _inputs(tmp_path)
    row = document["movements"][0]
    if change == "unknown":
        document["unknown"] = True
    elif change == "duplicate_account":
        document["equity_account_ids"].append("E")
    elif change == "duplicate_movement":
        document["movements"].append(dict(row))
    elif change == "both_sides":
        row["debit"] = "1"
    elif change == "zero_sides":
        row["credit"] = "0"
    elif change == "nan":
        row["credit"] = "NaN"
    elif change == "numeric_amount":
        row["credit"] = 25000
    elif change == "outside_date":
        row["date"] = "2026-08-31"
    elif change == "unknown_account":
        row["account_id"] = "X"
    elif change == "empty_evidence":
        row["evidence_reference"] = ""
    elif change == "schema_bool":
        document["schema_version"] = True
    elif change == "complete_string":
        document["complete"] = "true"
    elif change == "invalid_date":
        row["date"] = "2026-09-31"
    else:
        row["extra"] = "unwitnessed"
    _change(inputs, document)
    with pytest.raises(ControlInputError):
        review_close(**inputs)


def test_added_removed_or_non_equity_accounts_block(tmp_path):
    inputs, _ = _inputs(tmp_path, closing={"F": "129750"})
    assert review_close(**inputs).status == "BLOCKED"
    inputs, _ = _inputs(tmp_path, section="Liabilities")
    assert review_close(**inputs).status == "BLOCKED"


def test_absent_schedule_preserves_original_pack_shape(tmp_path):
    inputs, _ = _inputs(tmp_path)
    for key in ("equity_schedule_path", "equity_currency", "equity_currency_evidence"):
        inputs.pop(key)
    pack = review_close(**inputs)
    write_review_pack(pack, tmp_path / "pack")
    payload = verify_pack(tmp_path / "pack")[0]
    assert "equity_reconciliation" not in payload
    assert "equity_schedule" not in payload["source_sha256"]


def test_equity_json_and_markdown_tampering_fail_verification(tmp_path):
    inputs, _ = _inputs(tmp_path)
    output = tmp_path / "pack"
    pack = review_close(**inputs)
    write_review_pack(pack, output)
    path = output / "close-review-pack.json"
    document = json.loads(path.read_text(encoding="utf-8"))
    document["equity_reconciliation"]["movements"][0]["description"] = "Changed evidence"
    path.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ControlInputError):
        verify_pack(output)
    write_review_pack(pack, output)
    summary = output / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8").replace("130000", "99999"),
                       encoding="utf-8")
    with pytest.raises(ControlInputError):
        verify_pack(output)


def test_untrusted_movement_text_is_inert_in_summary(tmp_path):
    inputs, document = _inputs(tmp_path)
    document["movements"][0]["description"] = '<script>alert(1)</script> | **x**'
    _change(inputs, document)
    write_review_pack(review_close(**inputs), tmp_path / "pack")
    summary = verify_pack(tmp_path / "pack")[1]
    assert "<script>" not in summary
    assert "&lt;script&gt;" in summary


def test_low_decimal_context_cannot_round_equity_difference(tmp_path):
    inputs, _ = _inputs(tmp_path)
    with localcontext() as context:
        context.prec = 4
        result = review_close(**inputs)
    assert result.equity_reconciliation["total"]["unexplained"] == "-250"


def test_duplicate_json_field_is_refused(tmp_path):
    inputs, _ = _inputs(tmp_path)
    path = inputs["equity_schedule_path"]
    path.write_text(path.read_text(encoding="utf-8").replace(
        '"schema_version": 1', '"schema_version": 1, "schema_version": 1'), encoding="utf-8")
    with pytest.raises(ControlInputError, match="Duplicate"):
        review_close(**inputs)


def test_cli_status_and_malformed_input_preserve_existing_pack(tmp_path, capsys):
    inputs, document = _inputs(tmp_path)
    output = tmp_path / "pack"
    args = ["review", "--current", str(inputs["current_path"]), "--prior", str(inputs["prior_path"]),
            "--equity-schedule", str(inputs["equity_schedule_path"]),
            "--equity-currency", "AUD", "--equity-currency-evidence", "Both reports",
            "--output", str(output), "--absolute-threshold", "999999999"]
    assert main(args) == 2
    before = {path: path.read_bytes() for path in output.iterdir()}
    document["movements"][0]["credit"] = "NaN"
    _change(inputs, document)
    assert main(args) == 1
    assert {path: path.read_bytes() for path in before} == before
    assert "input error" in capsys.readouterr().err

