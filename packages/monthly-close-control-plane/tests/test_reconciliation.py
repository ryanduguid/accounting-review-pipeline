import csv
import json
from pathlib import Path

import pytest
from closecontrol.cli import main

COLUMNS = "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\n"


def write_transactions(path, rows):
    path.write_text(COLUMNS + "".join(
        f"Demo,clearing,AUD,{row}\n" for row in rows
    ), encoding="utf-8")
    return path


def run_pack(tmp_path, *, decisions=None, closing="-75.00", name="pack", extra=()):
    transactions = write_transactions(tmp_path / "transactions.csv", [
        "p1,2026-07-02,June,Settlement,0,100",
        "s1,2026-07-03,Batch,Sale,60,0",
        "s2,2026-07-03,Batch,Sale,40,0",
        "p2,2026-07-04,Batch,Settlement,0,100",
        "s3,2026-07-31,Pending,Sale,25,0",
    ])
    args = ["reconcile", "--transactions", str(transactions), "--tenant", "Demo",
            "--account-id", "clearing", "--currency", "AUD", "--period-start",
            "2026-07-01", "--period-end", "2026-07-31", "--opening-balance", "0",
            "--closing-balance", closing, "--output", str(tmp_path / name)]
    if decisions:
        args += ["--decisions", str(decisions)]
    args += list(extra)
    return main(args), tmp_path / name


def test_suggestions_do_not_clear_transactions(tmp_path):
    code, output = run_pack(tmp_path)
    assert code == 2
    pack = json.loads((output / "reconciliation.json").read_text())
    assert pack["status"] == "REVIEW"
    assert pack["outstanding_total"] == "-75.00"
    assert len(pack["outstanding"]) == 5
    assert pack["suggestions"] == [{"group": "S1", "ids": ["s1", "s2", "p2"],
                                    "reason": "Shared reference 'Batch'; total 0.00."}]
    assert (output / "review.html").exists()


def decision_file(tmp_path, rows):
    path = tmp_path / "decisions.csv"
    path.write_text("Group,TransactionID,Decision,Note\n" + "\n".join(rows) + "\n")
    return path


def test_reviewed_many_to_one_match_leaves_other_items(tmp_path):
    decisions = decision_file(tmp_path, [f"batch,{item},accept,Checked remittance"
                                        for item in ["s1", "s2", "p2"]])
    code, output = run_pack(tmp_path, decisions=decisions)
    pack = json.loads((output / "reconciliation.json").read_text())
    assert code == 2
    assert [row["TransactionID"] for row in pack["outstanding"]] == ["p1", "s3"]
    assert pack["outstanding_total"] == "-75.00"
    assert len(pack["decisions"]) == 1
    assert pack["decisions"][0]["note"] == "Checked remittance"


def test_reject_keeps_items_and_records_reason(tmp_path):
    decisions = decision_file(tmp_path, [f"batch,{item},reject,Unrelated activity"
                                        for item in ["s1", "s2", "p2"]])
    _, output = run_pack(tmp_path, decisions=decisions)
    pack = json.loads((output / "reconciliation.json").read_text())
    assert len(pack["outstanding"]) == 5
    assert pack["suggestions"] == []
    assert pack["decisions"][0]["decision"] == "reject"


@pytest.mark.parametrize("rows", [
    ["a,s1,accept,Checked"],
    ["a,unknown,accept,Checked", "a,s1,accept,Checked"],
    ["a,s1,accept,Checked", "a,s1,accept,Checked"],
    ["a,s1,accept,Checked", "a,p2,accept,Checked"],
    ["a,s1,accept,", "a,p2,accept,"],
    ["a,s1,accept,Checked", "a,s2,reject,Checked", "a,p2,accept,Checked"],
    ["a,s1,accept,Checked", "a,s2,accept,Other note", "a,p2,accept,Checked"],
    ["a,s1,accept,Checked", "a,s2,accept,Checked", "a,p2,accept,Checked",
     "b,s1,reject,Other group", "b,s2,reject,Other group"],
])
def test_invalid_decisions_fail_without_outputs(tmp_path, rows):
    code, output = run_pack(tmp_path, decisions=decision_file(tmp_path, rows))
    assert code == 1
    assert not output.exists()


def test_imbalance_blocks_carry_forward(tmp_path):
    code, output = run_pack(tmp_path, closing="-74.99")
    assert code == 2
    pack = json.loads((output / "reconciliation.json").read_text())
    assert pack["status"] == "BLOCKED"
    assert pack["closing_difference"] == "-0.01"
    assert not (output / "carry-forward.json").exists()


def test_opening_balance_requires_supporting_items(tmp_path):
    _, output = run_pack(tmp_path, closing="25", extra=["--opening-balance", "100"])
    pack = json.loads((output / "reconciliation.json").read_text())
    assert pack["status"] == "BLOCKED"
    assert pack["opening_difference"] == "-100.00"
    assert not (output / "carry-forward.json").exists()


def test_existing_output_and_source_files_survive(tmp_path):
    _, output = run_pack(tmp_path)
    before = {p.name: p.read_bytes() for p in output.iterdir()}
    code, _ = run_pack(tmp_path, closing="0")
    assert code == 1
    assert {p.name: p.read_bytes() for p in output.iterdir()} == before


@pytest.mark.parametrize("flag,value", [
    ("--period-end", "2026-06-30"), ("--period-start", "2026-07-03"),
    ("--period-start", "20260701"), ("--currency", "USD"),
    ("--tenant", "Other"), ("--account-id", "other"),
    ("--opening-balance", "NaN"), ("--closing-balance", "Infinity"),
    ("--closing-balance", "0.001"), ("--closing-balance", "1000000000000000"),
])
def test_invalid_configuration_or_identity_rejected(tmp_path, flag, value):
    code, output = run_pack(tmp_path, extra=[flag, value])
    assert code == 1
    assert not output.exists()


def test_changed_carry_forward_total_rejected(tmp_path):
    _, output = run_pack(tmp_path)
    carry_path = output / "carry-forward.json"
    carry = json.loads(carry_path.read_text())
    carry["balance"] = "999.00"
    carry_path.write_text(json.dumps(carry))
    with pytest.raises(ValueError, match="recorded balance"):
        compute(tmp_path / "transactions.csv", opening_items=carry_path,
                period_start="2026-08-01", period_end="2026-08-31")


def test_reordered_source_has_same_matching_suggestions(tmp_path):
    _, output = run_pack(tmp_path)
    # The source digest changes when rows change order, but group membership must not.
    from closecontrol.reconciliation import reconcile
    source = tmp_path / "transactions.csv"
    lines = source.read_text().splitlines()
    source.write_text(lines[0] + "\n" + "\n".join(reversed(lines[1:])) + "\n")
    pack = reconcile(source, tenant="Demo", account_id="clearing", currency="AUD",
                     period_start="2026-07-01", period_end="2026-07-31",
                     opening_balance="0", closing_balance="-75")
    assert pack["suggestions"][0]["ids"] == ["s1", "s2", "p2"]


def compute(source, **kwargs):
    from closecontrol.reconciliation import reconcile
    options = dict(tenant="Demo", account_id="clearing", currency="AUD",
                   period_start="2026-07-01", period_end="2026-07-31",
                   opening_balance="0", closing_balance="0")
    options.update(kwargs)
    return reconcile(source, **options)


@pytest.mark.parametrize("rows", [
    ["a,2026-07-01,R,Sale,1,0", "a,2026-07-02,R,Payment,0,1"],
    ["a,2026-07-01,R,Sale,NaN,0"], ["a,2026-07-01,R,Sale,0.001,0"],
    ["a,2026-07-01,R,Sale,,0"], ["a,2026-07-01,R,Sale,1"],
    ["a,2026-07-01,R,Sale,1,1"], ["a,2026-07-01,R,Sale,0,0"],
    ["a,2026-07-01,R,Sale,-1,0"], ["=ID,2026-07-01,R,Sale,1,0"],
    ["a,2026-02-30,R,Sale,1,0"], ["a,2026-07-01,R,Sale,1,0,extra"],
])
def test_bad_transactions_rejected(tmp_path, rows):
    with pytest.raises(ValueError):
        compute(write_transactions(tmp_path / "bad.csv", rows))


def test_equal_amounts_without_shared_reference_not_suggested(tmp_path):
    source = write_transactions(tmp_path / "different.csv", [
        "a,2026-07-01,Customer A,Sale,500,0", "b,2026-07-01,Customer B,Payment,0,500"])
    pack = compute(source)
    assert pack["status"] == "REVIEW"
    assert pack["suggestions"] == []
    assert len(pack["outstanding"]) == 2


def test_manual_many_to_many_match_can_cross_references(tmp_path):
    source = write_transactions(tmp_path / "manual.csv", [
        "a,2026-07-01,A,Sale,60,0", "b,2026-07-01,B,Sale,40,0",
        "c,2026-07-02,C,Payment,0,30", "d,2026-07-02,D,Payment,0,70"])
    decisions = decision_file(tmp_path, [f"manual,{key},accept,Traced bank batch" for key in "abcd"])
    pack = compute(source, decisions=decisions)
    assert pack["status"] == "PASS"
    assert pack["outstanding"] == []


def test_host_decimal_context_does_not_round_money(tmp_path):
    from decimal import localcontext
    source = write_transactions(tmp_path / "precise.csv", [
        "a,2026-07-01,A,Sale,999999999999999.99,0", "b,2026-07-01,B,Payment,0,999999999999999.98"])
    with localcontext() as context:
        context.prec = 2
        pack = compute(source, closing_balance="0.01")
    assert pack["outstanding_total"] == "0.01"
    assert pack["closing_difference"] == "0.00"


def test_html_and_csv_keep_source_text_inert_and_json_keeps_original(tmp_path):
    from closecontrol.reconciliation_report import write_reconciliation
    source = write_transactions(tmp_path / "hostile.csv", [
        'a,2026-07-01,=1+1,<script>alert(1)</script>,1,0',
        'b,2026-07-02,other,Payment,0,1'])
    decisions = decision_file(tmp_path, ["a,a,reject,=1+1", "a,b,reject,=1+1"])
    pack = compute(source, decisions=decisions)
    output = tmp_path / "safe"
    write_reconciliation(pack, output)
    html = (output / "review.html").read_text()
    assert "<script>alert(1)</script>" not in html
    assert "&lt;script&gt;alert(1)&lt;/script&gt;" in html
    with (output / "outstanding.csv").open(encoding="utf-8-sig", newline="") as stream:
        records = list(csv.DictReader(stream))
    assert records[0]["Reference"] == "'=1+1"
    assert records[0]["ReviewNote"] == "'=1+1"
    assert records[0]["AgeDays"] == "30"
    carry = json.loads((output / "carry-forward.json").read_text())
    assert carry["items"][0]["Reference"] == "=1+1"
    assert carry["notes"]["a"] == "=1+1"


def test_failed_write_removes_only_new_pack(tmp_path, monkeypatch):
    from closecontrol.reconciliation_report import write_reconciliation
    source = write_transactions(tmp_path / "write.csv", ["a,2026-07-01,A,Sale,1,0"])
    pack = compute(source, closing_balance="1")
    original_open = Path.open

    def fail_carry(path, *args, **kwargs):
        if path.name == "carry-forward.json":
            raise PermissionError("test locked file")
        return original_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", fail_carry)
    with pytest.raises(PermissionError):
        write_reconciliation(pack, tmp_path / "failed")
    assert not (tmp_path / "failed").exists()
    assert source.exists()


def test_output_inside_repository_rejected_before_source_read(tmp_path, capsys):
    code, _ = run_pack(tmp_path, extra=["--transactions", str(tmp_path / "absent.csv"),
                                      "--output", str(Path(__file__).parent / "bad-pack")])
    assert code == 1
    error = capsys.readouterr().err
    assert "version-control" in error
    assert "does not exist" not in error


def test_three_month_worked_example(tmp_path):
    import subprocess
    import sys
    script = Path(__file__).parents[1] / "examples" / "clearing_demo.py"
    completed = subprocess.run([sys.executable, str(script), "--output", str(tmp_path / "demo")],
                               capture_output=True, text=True)
    assert completed.returncode == 0, completed.stdout + completed.stderr
    june = json.loads((tmp_path / "demo/june/reconciliation.json").read_text())
    july = json.loads((tmp_path / "demo/july-reviewed/reconciliation.json").read_text())
    august = json.loads((tmp_path / "demo/august/reconciliation.json").read_text())
    assert (june["status"], june["outstanding_total"]) == ("REVIEW", "100.00")
    assert (july["status"], july["outstanding_total"]) == ("REVIEW", "25.00")
    assert july["outstanding"][0]["TransactionID"] == "s3"
    assert (august["status"], august["outstanding_total"]) == ("PASS", "0.00")
    assert august["outstanding"] == []


def test_rejected_notes_and_dates_survive_next_month(tmp_path):
    from closecontrol.reconciliation_report import write_reconciliation
    source = write_transactions(tmp_path / "july.csv", [
        "a,2026-07-01,A,Sale,10,0", "b,2026-07-02,B,Payment,0,10"])
    decisions = decision_file(tmp_path, ["not-related,a,reject,Needs investigation",
                                         "not-related,b,reject,Needs investigation"])
    pack = compute(source, decisions=decisions)
    write_reconciliation(pack, tmp_path / "july")
    empty = write_transactions(tmp_path / "august.csv", [])
    august = compute(empty, opening_items=tmp_path / "july/carry-forward.json",
                     period_start="2026-08-01", period_end="2026-08-31")
    assert august["status"] == "REVIEW"
    assert august["outstanding"][0]["Date"] == "2026-07-01"
    assert august["notes"] == {"a": "Needs investigation", "b": "Needs investigation"}


@pytest.mark.parametrize("mutation, message", [
    (lambda p: p.update(schema="other"), "invalid schema"),
    (lambda p: p.update(identity={}), "invalid schema"),
    (lambda p: p.update(period_end="2026-07-30"), "immediately preceding"),
    (lambda p: p["items"][0].update(Date="2026-08-01"), "later than"),
    (lambda p: p.update(items=[None]), "transaction objects"),
    (lambda p: p.update(notes={"unknown": "note"}), "must name outstanding"),
    (lambda p: p.update(notes={"p1": 123}), "must name outstanding"),
])
def test_carry_forward_boundaries(tmp_path, mutation, message):
    _, output = run_pack(tmp_path)
    path = output / "carry-forward.json"
    carry = json.loads(path.read_text())
    mutation(carry)
    path.write_text(json.dumps(carry))
    with pytest.raises(ValueError, match=message):
        compute(tmp_path / "transactions.csv", opening_items=path,
                period_start="2026-08-01", period_end="2026-08-31")


def test_duplicate_id_across_months_rejected(tmp_path):
    _, output = run_pack(tmp_path)
    source = write_transactions(tmp_path / "august.csv", ["p1,2026-08-01,A,Duplicate,100,0"])
    with pytest.raises(ValueError, match="Repeated TransactionID"):
        compute(source, opening_items=output / "carry-forward.json", opening_balance="-75",
                closing_balance="25", period_start="2026-08-01", period_end="2026-08-31")


def test_generated_decisions_can_be_reused_without_losing_allocations(tmp_path):
    decisions = decision_file(tmp_path, [f"S1,{item},reject,Unrelated activity"
                                        for item in ["p1", "s3"]])
    _, output = run_pack(tmp_path, decisions=decisions)
    with (output / "suggestions.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    assert {r["Group"] for r in rows} == {"S1", "new-S1"}
    second = compute(tmp_path / "transactions.csv", closing_balance="-75",
                     decisions=output / "suggestions.csv")
    assert second["decisions"][0]["decision"] == "reject"
    assert len(second["outstanding"]) == 5


@pytest.mark.parametrize("note", ["", "Awaiting remittance"])
@pytest.mark.parametrize("reference", ["Shared", "Other"])
def test_pending_groups_survive_reruns_without_clearing_or_duplicate_suggestions(tmp_path, note, reference):
    from closecontrol.reconciliation_report import write_reconciliation
    source = write_transactions(tmp_path / "pending.csv", [
        "a,2026-07-01,Shared,Receipt,10,0",
        f"b,2026-07-02,{reference},Payment,0,10",
        "c,2026-07-03,Later,Unsettled,5,0",
    ])
    decisions = decision_file(tmp_path, [f"manual,{key},,{note}" for key in "ab"])
    pack = compute(source, closing_balance="5", decisions=decisions)
    write_reconciliation(pack, tmp_path / "first")
    with (tmp_path / "first/suggestions.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    assert rows == [{"Group": "manual", "TransactionID": key, "Decision": "", "Note": note}
                    for key in "ab"]
    second = compute(source, closing_balance="5", decisions=tmp_path / "first/suggestions.csv")
    assert second["status"] == "REVIEW"
    assert len(second["outstanding"]) == 3
    assert second["suggestions"] == []
    assert second["decisions"] == [{"group": "manual", "ids": ["a", "b"], "decision": "", "note": note}]
    assert "<strong>pending</strong>" in (tmp_path / "first/review.html").read_text()
    empty = write_transactions(tmp_path / "empty.csv", [])
    next_month = compute(empty, opening_balance="5", closing_balance="5",
                         opening_items=tmp_path / "first/carry-forward.json",
                         period_start="2026-08-01", period_end="2026-08-31")
    assert next_month["notes"] == ({"a": note, "b": note} if note else {})
    assert len(next_month["outstanding"]) == 3


def test_partial_settlements_roll_forward_without_clearing_unrelated_zero_net_items(tmp_path):
    # Catches clearing partial groups, losing their original lines, or treating net zero as PASS.
    from closecontrol.reconciliation_report import write_reconciliation
    july_source = write_transactions(tmp_path / "july.csv", [
        "sale,2026-07-01,Partial,Sale,100,0",
        "part,2026-07-02,Partial,Part payment,0,60",
        "original,2026-07-03,Reversal,Original posting,20,0",
        "reversal,2026-07-04,Reversal,Full reversal,0,20",
        "batch,2026-07-05,Batch,Gross settlement receivable,200,0",
        "payout,2026-07-06,Batch,Net settlement received,0,197",
        "unrelated-dr,2026-07-10,Customer-A,Unresolved receipt,50,0",
        "unrelated-cr,2026-07-10,Customer-B,Unresolved payment,0,50",
    ])
    july_decisions = decision_file(tmp_path, [
        "reversed,original,accept,Checked original and full reversal",
        "reversed,reversal,accept,Checked original and full reversal",
    ])
    july = compute(july_source, closing_balance="43", decisions=july_decisions)
    assert july["status"] == "REVIEW"
    assert july["outstanding_total"] == "43.00"
    assert july["suggestions"] == []
    assert {r["TransactionID"] for r in july["outstanding"]} == {
        "sale", "part", "batch", "payout", "unrelated-dr", "unrelated-cr"}
    write_reconciliation(july, tmp_path / "july")

    august_source = write_transactions(tmp_path / "august.csv", [
        "final,2026-08-02,Partial,Final payment,0,40",
        "fee,2026-08-03,Batch,Fee already posted in source ledger,0,3",
    ])
    august_decisions = decision_file(tmp_path, [
        "settled,sale,accept,Checked both payments against the receipt",
        "settled,part,accept,Checked both payments against the receipt",
        "settled,final,accept,Checked both payments against the receipt",
        "batch,batch,accept,Checked net payout and source fee posting",
        "batch,payout,accept,Checked net payout and source fee posting",
        "batch,fee,accept,Checked net payout and source fee posting",
    ])
    august = compute(august_source, opening_balance="43", decisions=august_decisions,
                     opening_items=tmp_path / "july/carry-forward.json",
                     period_start="2026-08-01", period_end="2026-08-31")
    assert august["movement"] == "-43.00"
    assert august["closing_difference"] == "0.00"
    assert august["outstanding_total"] == "0.00"
    assert august["status"] == "REVIEW"
    assert {r["TransactionID"] for r in august["outstanding"]} == {"unrelated-dr", "unrelated-cr"}
    write_reconciliation(august, tmp_path / "august")
    with (tmp_path / "august/outstanding.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    assert [r["AgeDays"] for r in rows] == ["52", "52"]
