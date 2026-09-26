"""review-ready compare: two verified runs of one pack, and what moved between them."""

from __future__ import annotations

import json
from dataclasses import replace
from decimal import Decimal
from pathlib import Path

import pytest
from reviewready.cli import main
from reviewready.comparison import COMPARISON_BOUNDARY, compare_packs
from reviewready.engine import ReadinessPack, review_pack
from reviewready.errors import GateInputError
from reviewready.models import ControlNotRun, Finding, SourceEvidence
from reviewready.report import write_review_pack
from tests.support import EXAMPLES


def _run(example: str, output: Path, **overrides: object) -> Path:
    """Gate one fabricated example pack into output and return the directory."""
    write_review_pack(review_pack(profile="bas", pack_dir=EXAMPLES / example, **overrides), output)
    return output


def _finding(reason: str) -> Finding:
    return Finding(
        code="MISSING_ARTEFACT",
        status="NOT_READY",
        slot="gst_control_gl",
        reason=reason,
        reviewer_action="Return the pack to the preparer. Do not start technical review.",
    )


def _synthetic(findings: tuple[Finding, ...], digest: str = "a" * 64) -> ReadinessPack:
    return ReadinessPack(
        status="NOT_READY" if findings else "READY",
        engagement_type="bas",
        period_end="2026-03-31",
        preparer_initials="AB",
        findings=findings,
        source_evidence=(SourceEvidence("trial_balance", "trial_balance.csv", digest),),
        tieout_tolerance=Decimal("0.01"),
        acknowledgement=None,
    )


def _by_key(rows: list[dict], *keys: str) -> dict[tuple, dict]:
    return {tuple(row[key] for key in keys): row for row in rows}


def test_same_run_twice_recurs_and_changes_nothing(tmp_path: Path) -> None:
    first = _run("bas-not-ready", tmp_path / "first")
    second = _run("bas-not-ready", tmp_path / "second")

    result = compare_packs(first, second)

    assert result["previous"]["overall_status"] == result["current"]["overall_status"] == "NOT_READY"
    assert result["scope_changes"] == []
    assert {row["change"] for row in result["sources"]} == {"UNCHANGED"}
    assert result["findings"]
    assert {row["change"] for row in result["findings"]} == {"RECURRING"}
    assert result["comparison_boundary"] == COMPARISON_BOUNDARY


def test_fixed_pack_shows_changed_sources_and_findings_not_raised(tmp_path: Path) -> None:
    before = _run("bas-not-ready", tmp_path / "before")
    after = _run("bas-ready", tmp_path / "after")

    result = compare_packs(before, after)

    assert result["previous"]["overall_status"] == "NOT_READY"
    assert result["current"]["overall_status"] == "READY"
    assert "CHANGED" in {row["change"] for row in result["sources"]}
    # The fixed pack skips the prior-findings control, and that coverage
    # change is named even though none of the missing findings sat in it.
    assert [change["member"] for change in result["scope_changes"]] == ["controls_not_run"]
    # Absent from the later run is reported as not raised, never as resolved.
    assert {row["change"] for row in result["findings"]} == {"NOT_RAISED"}
    assert all(row["current"] == [] and row["previous"] for row in result["findings"])


def test_a_changed_tolerance_makes_absent_findings_not_comparable(tmp_path: Path) -> None:
    before = _run("bas-not-ready", tmp_path / "before")
    after = _run("bas-ready", tmp_path / "after", tieout_tolerance=Decimal("5.00"))

    result = compare_packs(before, after)

    assert [change["member"] for change in result["scope_changes"]] == ["thresholds", "controls_not_run"]
    assert {row["change"] for row in result["findings"]} == {"NOT_COMPARABLE"}


def test_a_finding_in_a_slot_the_later_run_skipped_is_not_comparable(tmp_path: Path) -> None:
    before = tmp_path / "before"
    after = tmp_path / "after"
    skipped = replace(_finding("Bank reconciliation is empty."), code="EMPTY_ARTEFACT", slot="bank_rec")
    write_review_pack(_synthetic((_finding("Still missing."), skipped)), before)
    write_review_pack(
        replace(
            _synthetic((_finding("Still missing."),)),
            controls_not_run=(ControlNotRun("bank_rec", "bank_rec.csv", "no file was supplied"),),
        ),
        after,
    )

    rows = _by_key(compare_packs(before, after)["findings"], "code", "slot")

    assert rows[("EMPTY_ARTEFACT", "bank_rec")]["change"] == "NOT_COMPARABLE"
    assert rows[("MISSING_ARTEFACT", "gst_control_gl")]["change"] == "RECURRING"


def test_a_later_pack_that_states_no_coverage_is_not_comparable(tmp_path: Path) -> None:
    before = tmp_path / "before"
    after = tmp_path / "after"
    # The earlier run states full coverage. The later one omits the member, as a
    # pack written before coverage was recorded does, so nothing shows the control ran.
    write_review_pack(replace(_synthetic((_finding("Still missing."),)), controls_not_run=()), before)
    write_review_pack(_synthetic(()), after)

    result = compare_packs(before, after)

    assert result["scope_changes"] == [{"member": "controls_not_run", "previous": [], "current": None}]
    assert {row["change"] for row in result["findings"]} == {"NOT_COMPARABLE"}


def test_reordered_coverage_is_not_a_scope_change(tmp_path: Path) -> None:
    before = tmp_path / "before"
    after = tmp_path / "after"
    skipped = (
        ControlNotRun("bank_rec", "bank_rec.csv", "no file was supplied"),
        ControlNotRun("prior_findings", "prior_findings.csv", "no file was supplied"),
    )
    write_review_pack(replace(_synthetic(()), controls_not_run=skipped), before)
    write_review_pack(replace(_synthetic(()), controls_not_run=skipped[::-1]), after)

    assert compare_packs(before, after)["scope_changes"] == []


def test_an_equivalent_tolerance_is_not_a_scope_change(tmp_path: Path) -> None:
    before = _run("bas-not-ready", tmp_path / "before")
    after = _run("bas-ready", tmp_path / "after", tieout_tolerance=Decimal("0.010"))
    written = json.loads((after / "readiness-pack.json").read_text(encoding="utf-8"))
    assert written["thresholds"] == {"tieout_tolerance": "0.010"}

    result = compare_packs(before, after)

    assert [change["member"] for change in result["scope_changes"]] == ["controls_not_run"]
    assert {row["change"] for row in result["findings"]} == {"NOT_RAISED"}


def test_new_and_changed_groups_are_named(tmp_path: Path) -> None:
    before = tmp_path / "before"
    after = tmp_path / "after"
    write_review_pack(_synthetic((_finding("First reason."),)), before)
    extra = replace(_finding("Another slot."), slot="bank_rec", code="EMPTY_ARTEFACT")
    write_review_pack(_synthetic((_finding("Second reason."), extra)), after)

    rows = _by_key(compare_packs(before, after)["findings"], "code", "slot")

    assert rows[("MISSING_ARTEFACT", "gst_control_gl")]["change"] == "CHANGED"
    assert rows[("EMPTY_ARTEFACT", "bank_rec")]["change"] == "NEW"
    assert rows[("EMPTY_ARTEFACT", "bank_rec")]["previous"] == []


def test_a_changed_source_digest_is_reported_per_slot(tmp_path: Path) -> None:
    before = tmp_path / "before"
    after = tmp_path / "after"
    write_review_pack(_synthetic((), digest="a" * 64), before)
    write_review_pack(_synthetic((), digest="b" * 64), after)

    rows = _by_key(compare_packs(before, after)["sources"], "slot")

    assert rows[("trial_balance",)]["change"] == "CHANGED"
    assert rows[("trial_balance",)]["previous"]["sha256"] == "a" * 64
    assert rows[("trial_balance",)]["current"]["sha256"] == "b" * 64


def test_packs_for_another_engagement_are_refused(tmp_path: Path) -> None:
    before = _run("bas-not-ready", tmp_path / "before")
    month_end = tmp_path / "month-end"
    write_review_pack(review_pack(profile="month_end", pack_dir=EXAMPLES / "month-end-ready"), month_end)

    with pytest.raises(GateInputError, match="same engagement type"):
        compare_packs(before, month_end)


def test_cli_prints_sorted_json_and_exits_zero(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    before = _run("bas-not-ready", tmp_path / "before")
    after = _run("bas-ready", tmp_path / "after")

    code = main(["compare", "--previous-pack-dir", str(before), "--current-pack-dir", str(after)])

    out = capsys.readouterr().out
    assert code == 0
    assert json.loads(out) == compare_packs(before, after)
    assert out == json.dumps(json.loads(out), indent=2, sort_keys=True) + "\n"


def test_cli_fails_closed_on_an_edited_pack(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    before = _run("bas-not-ready", tmp_path / "before")
    after = _run("bas-ready", tmp_path / "after")
    document = json.loads((after / "readiness-pack.json").read_text(encoding="utf-8"))
    document["overall_status"] = "NOT_READY"
    (after / "readiness-pack.json").write_text(json.dumps(document, indent=2, sort_keys=True) + "\n",
                                                encoding="utf-8")

    code = main(["compare", "--previous-pack-dir", str(before), "--current-pack-dir", str(after)])

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == ""
    assert "review-ready compare: verification failed" in captured.err
