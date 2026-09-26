import json
from decimal import Decimal

import pytest
from closecontrol.cli import main
from closecontrol.comparison import compare_packs
from closecontrol.engine import review_close
from closecontrol.errors import ControlInputError
from closecontrol.report import write_review_pack

HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"


def _tb(root, name, when, tenant="Synthetic"):
    path = root / (name + ".csv")
    path.write_text(HEADER + "\n"
                    + f"{when},{tenant},Assets,A,Bank,090,100,0,100,0\n"
                    + f"{when},{tenant},Equity,E,Capital,300,0,100,0,100\n", encoding="utf-8")
    return path


def _pair(tmp_path, *, omit=False, amount="90", tenant="Synthetic", threshold="1000"):
    july = _tb(tmp_path, "july", "2026-07-31")
    august = _tb(tmp_path, "august", "2026-08-31")
    september = _tb(tmp_path, "september", "2026-09-30", tenant)
    subledger = tmp_path / "subledger.csv"
    subledger.write_text("Tenant,AccountID,SubledgerBalance\nSynthetic,A,90\n", encoding="utf-8")
    old = tmp_path / "old"
    write_review_pack(review_close(current_path=august, prior_path=july,
                                   subledger_path=subledger), old)
    subledger.write_text(f"Tenant,AccountID,SubledgerBalance\n{tenant},A,{amount}\n", encoding="utf-8")
    if tenant != "Synthetic":
        august = _tb(tmp_path, "other-august", "2026-08-31", tenant)
    new = tmp_path / "new"
    write_review_pack(review_close(current_path=september, prior_path=august,
                                   subledger_path=None if omit else subledger,
                                   absolute_threshold=Decimal(threshold)), new)
    return dict(previous_pack=old, current_pack=new,
                previous_tb=tmp_path / "august.csv", current_tb=september)


def test_repeat_query_keeps_identity_and_sources_are_unchanged(tmp_path):
    inputs = _pair(tmp_path)
    before = {p: p.read_bytes() for p in tmp_path.rglob("*") if p.is_file()}
    result = compare_packs(**inputs)
    assert result["scope_changes"] == []
    assert result["findings"][0]["change"] == "RECURRING"
    assert result["queries"][0]["change"] == "RECURRING"
    assert {p: p.read_bytes() for p in before} == before
    assert result["current_status"] == "REVIEW"


def test_amount_change_preserves_before_and_after(tmp_path):
    result = compare_packs(**_pair(tmp_path, amount="80"))
    item = result["findings"][0]
    assert item["change"] == "CHANGED"
    assert item["previous"][0]["difference"] == "10.00"
    assert item["current"][0]["difference"] == "20.00"


def test_omitted_control_is_not_resolution(tmp_path):
    result = compare_packs(**_pair(tmp_path, omit=True))
    assert result["current_status"] == "PASS"
    assert "controls_not_run" in result["scope_changes"]
    assert result["findings"][0]["change"] == "NOT_COMPARABLE"
    assert result["queries"][0]["change"] == "NOT_COMPARABLE"


def test_no_longer_raised_is_not_approved(tmp_path):
    inputs = _pair(tmp_path)
    current = inputs["current_tb"]
    current.write_text(current.read_text().replace(",100,", ",90,").replace(",100\n", ",90\n"))
    replacement = tmp_path / "replacement"
    write_review_pack(review_close(current_path=current, prior_path=inputs["previous_tb"],
                                   subledger_path=tmp_path / "subledger.csv"), replacement)
    inputs["current_pack"] = replacement
    result = compare_packs(**inputs)
    assert result["findings"][0]["change"] == "NOT_RAISED"
    assert "does not mean resolved" in result["review_boundary"]


def test_changed_subledger_file_does_not_prove_unchanged_coverage(tmp_path):
    result = compare_packs(**_pair(tmp_path, amount="100"))
    assert "subledger_population_not_verified" in result["scope_changes"]
    assert result["findings"][0]["change"] == "NOT_COMPARABLE"


def test_threshold_change_keeps_absent_finding_unresolved(tmp_path):
    result = compare_packs(**_pair(tmp_path, amount="100", threshold="2000"))
    assert "thresholds" in result["scope_changes"]
    assert result["findings"][0]["change"] == "NOT_COMPARABLE"


def test_tampered_pack_and_wrong_source_are_refused(tmp_path):
    inputs = _pair(tmp_path)
    original = inputs["current_tb"].read_bytes()
    inputs["current_tb"].write_bytes(original + b"\n")
    with pytest.raises(ControlInputError, match="does not match"):
        compare_packs(**inputs)
    inputs["current_tb"].write_bytes(original)
    summary = inputs["current_pack"] / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8").replace("REVIEW", "PASS"),
                       encoding="utf-8")
    with pytest.raises(ControlInputError):
        compare_packs(**inputs)


def test_different_entities_are_refused_even_when_balances_match(tmp_path):
    with pytest.raises(ControlInputError, match="same tenant"):
        compare_packs(**_pair(tmp_path, tenant="Another entity"))


def test_response_is_bound_to_current_pack_and_never_clears_findings(tmp_path):
    inputs = _pair(tmp_path)
    base = compare_packs(**inputs)
    response = tmp_path / "responses.json"
    document = {"schema_version": 1,
                "pack_sha256": base["current_artefact_sha256"]["close-review-pack.json"],
                "responses": [{"query_id": base["queries"][0]["query_id"],
                               "explanation": "Timing difference",
                               "evidence_reference": "Synthetic statement item 4",
                               "reviewer": "AB", "reviewed_on": "2026-10-01"}]}
    response.write_text(json.dumps(document), encoding="utf-8")
    result = compare_packs(**inputs, responses=response)
    assert result["current_status"] == "REVIEW"
    assert result["findings"] == base["findings"]
    assert len(result["responses"]) == 1
    document["pack_sha256"] = "0" * 64
    response.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ControlInputError, match="bind"):
        compare_packs(**inputs, responses=response)


@pytest.mark.parametrize("change", ["duplicate", "unknown", "date", "blank", "extra"])
def test_invalid_response_is_rejected(tmp_path, change):
    inputs = _pair(tmp_path)
    base = compare_packs(**inputs)
    row = {"query_id": base["queries"][0]["query_id"], "explanation": "Check",
           "evidence_reference": "Item", "reviewer": "AB", "reviewed_on": "2026-10-01"}
    if change == "unknown":
        row["query_id"] = "unknown"
    if change == "date":
        row["reviewed_on"] = "2026-09-01"
    if change == "blank":
        row["explanation"] = " "
    if change == "extra":
        row["approved"] = "yes"
    document = {"schema_version": 1,
                "pack_sha256": base["current_artefact_sha256"]["close-review-pack.json"],
                "responses": [row, row] if change == "duplicate" else [row]}
    path = tmp_path / "response.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    with pytest.raises(ControlInputError):
        compare_packs(**inputs, responses=path)


# One small recurring mis-coding: every month 150 of software subscriptions is
# posted to Office Expenses (610) instead of Software Subscriptions (620).
MISCODED = Decimal("150")
MONTH_ENDS = ("2025-07-31", "2025-08-31", "2025-09-30", "2025-10-31", "2025-11-30",
              "2025-12-31", "2026-01-31", "2026-02-28", "2026-03-31", "2026-04-30",
              "2026-05-31", "2026-06-30")


def _drift_comparisons(tmp_path, threshold):
    """Close 12 fabricated month-ends in one financial year and compare each pair of packs."""
    movement = {"100": ("Assets", "Operating Bank", Decimal("18850")),
                "300": ("Revenue", "Sales Revenue", Decimal("-20000")),
                "610": ("Expenses", "Office Expenses", Decimal("600") + MISCODED),
                "620": ("Expenses", "Software Subscriptions", Decimal("550") - MISCODED),
                "900": ("Equity", "Retained Earnings", Decimal("0"))}
    opening = {"100": Decimal("50000"), "900": Decimal("-50000")}
    ledger = []
    for month, when in enumerate(MONTH_ENDS, start=1):
        lines = [HEADER]
        for account, (section, name, amount) in movement.items():
            ytd = opening.get(account, Decimal("0")) + amount * month
            debit, credit = (amount, 0) if amount > 0 else (0, abs(amount))
            ytd_debit, ytd_credit = (ytd, 0) if ytd > 0 else (0, abs(ytd))
            lines.append(f"{when},Synthetic,{section},{account},{name},{account},"
                         f"{debit},{credit},{ytd_debit},{ytd_credit}")
        path = tmp_path / f"{when}.csv"
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        ledger.append(path)
    packs = []
    for prior, current in zip(ledger, ledger[1:]):
        pack = tmp_path / f"pack-{current.stem}"
        write_review_pack(review_close(current_path=current, prior_path=prior,
                                       absolute_threshold=Decimal(threshold)), pack)
        packs.append((pack, current))
    return [compare_packs(previous_pack=old, previous_tb=old_tb, current_pack=new, current_tb=new_tb)
            for (old, old_tb), (new, new_tb) in zip(packs, packs[1:])]


def test_small_recurring_miscoding_never_becomes_a_finding(tmp_path):
    comparisons = _drift_comparisons(tmp_path, "1000")
    assert len(comparisons) == 10
    assert all(result["scope_changes"] == [] for result in comparisons)
    named = {item["account_id"] for result in comparisons for item in result["findings"]}
    assert named == {"100", "300"}
    # By June, Office Expenses is overstated by 12 x 150 and the close still passes.
    assert comparisons[-1]["current_status"] == "PASS"


def test_lower_threshold_cannot_tell_the_miscoded_account_apart(tmp_path):
    comparisons = _drift_comparisons(tmp_path, "100")

    def changes(account):
        return [item["change"] for result in comparisons
                for item in result["findings"] if item["account_id"] == account]

    assert changes("610") == changes("620") == ["CHANGED"] * 9 + ["NOT_RAISED"]


def test_cli_prints_verified_json_and_error_is_exit_one(tmp_path, capsys):
    inputs = _pair(tmp_path)
    args = ["compare"]
    for key, path in inputs.items():
        args.extend(["--" + key.replace("_", "-"), str(path)])
    assert main(args) == 0
    assert json.loads(capsys.readouterr().out)["current_status"] == "REVIEW"
    inputs["current_tb"].write_text("invalid", encoding="utf-8")
    assert main(args) == 1
    assert "verification failed" in capsys.readouterr().err
