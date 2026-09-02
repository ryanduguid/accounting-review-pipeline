from __future__ import annotations

import json
from pathlib import Path

from reviewready.engine import review_pack

ROOT = Path(__file__).resolve().parent.parent
PACK = ROOT / "evaluation" / "manager_review_gate"
EXPECTED = PACK / "expected_results.json"


def test_manager_review_evaluation_reproduces_the_declared_results() -> None:
    contract = json.loads(EXPECTED.read_text(encoding="utf-8"))
    assert contract["schema_version"] == 1
    assert contract["product_release"] == "0.1.2"
    assert contract["fixture_version"] == "1"
    for scenario in contract["scenarios"]:
        first_result = review_pack(
            profile=scenario["profile"],
            pack_dir=ROOT / scenario["fixture"],
        )
        second_result = review_pack(
            profile=scenario["profile"],
            pack_dir=ROOT / scenario["fixture"],
        )
        assert first_result.status == scenario["expected_status"]
        assert second_result.status == scenario["expected_status"]
        assert [item.code for item in first_result.findings] == scenario["expected_findings"]
        assert [item.code for item in second_result.findings] == scenario["expected_findings"]


def test_manager_review_evaluation_keeps_the_human_decision_visible() -> None:
    contract = json.loads(EXPECTED.read_text(encoding="utf-8"))
    readme = (PACK / "README.md").read_text(encoding="utf-8")
    boundary = contract["human_decision"]
    assert boundary == (
        "READY means no configured gate tripped; it is not approval, not a tax or "
        "BAS agent service, not advice and not lodgment authority. A human remains "
        "accountable for professional judgement, approval, client impact and lodgment "
        "decisions."
    )
    assert boundary in readme
    for clause in (
        "not approval",
        "not a tax or BAS agent service",
        "not advice",
        "not lodgment authority",
        "A human remains accountable for professional judgement, approval, client impact "
        "and lodgment decisions",
    ):
        assert clause in boundary
        assert clause in readme
    assert "fabricated" in readme.casefold()
    assert "client workpaper" in readme.casefold()
    assert "case study" not in readme.casefold()


def test_manager_review_evaluation_names_sources_and_review_date() -> None:
    contract = json.loads(EXPECTED.read_text(encoding="utf-8"))
    assert contract["source_reviewed"] == "2026-08-26"
    assert {source["publisher"] for source in contract["sources"]} == {
        "Australian Taxation Office"
    }
    assert all(
        source["url"].startswith("https://www.ato.gov.au/")
        for source in contract["sources"]
    )
