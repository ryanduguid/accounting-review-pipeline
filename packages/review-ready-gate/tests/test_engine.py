from __future__ import annotations

import json
from decimal import Decimal
from pathlib import Path

import pytest
from reviewready.cli import main
from reviewready.engine import review_pack
from reviewready.models import (
    FINDING_EMPTY_ARTEFACT,
    FINDING_OPEN_ITEM_INCOMPLETE,
    FINDING_PERIOD_ORDER,
    FINDING_SELF_REVIEW_INCOMPLETE,
)
from tests.support import EXAMPLES, copy_example_pack


def test_empty_required_artefact_is_not_ready(tmp_path: Path) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    (dest / "gst_control_gl.csv").write_bytes(b"")
    pack = review_pack(profile="bas", pack_dir=dest)
    assert pack.status == "NOT_READY"
    assert any(
        item.code == FINDING_EMPTY_ARTEFACT and item.slot == "gst_control_gl"
        for item in pack.findings
    )


def test_false_self_review_assertion_is_not_ready(tmp_path: Path) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    payload = json.loads((dest / "self_review.json").read_text(encoding="utf-8"))
    payload["assertions"]["pack_complete"] = False
    (dest / "self_review.json").write_text(json.dumps(payload), encoding="utf-8")
    pack = review_pack(profile="bas", pack_dir=dest)
    assert pack.status == "NOT_READY"
    assert any(item.code == FINDING_SELF_REVIEW_INCOMPLETE for item in pack.findings)


def test_cleared_open_item_without_resolution_is_not_ready(tmp_path: Path) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    (dest / "open_items.csv").write_text(
        "ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n"
        "OI-001,EXPLAIN,AB,2026-04-05,CLEARED,Fuel tax credit worksheet arrived late,\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="bas", pack_dir=dest)
    assert pack.status == "NOT_READY"
    assert any(item.code == FINDING_OPEN_ITEM_INCOMPLETE for item in pack.findings)


def test_tenant_mismatch_is_blocked(tmp_path: Path) -> None:
    dest = copy_example_pack("month-end-ready", tmp_path / "pack")
    prior = dest / "prior_trial_balance.csv"
    prior.write_text(
        prior.read_text(encoding="utf-8").replace(
            "Varrock Ventures Pty Ltd", "Cedar and Pine Consulting Pty Ltd"
        ),
        encoding="utf-8",
    )
    pack = review_pack(profile="month_end", pack_dir=dest)
    assert pack.status == "BLOCKED"
    assert any(item.code == FINDING_PERIOD_ORDER for item in pack.findings)


def test_prior_date_not_earlier_is_blocked(tmp_path: Path) -> None:
    dest = copy_example_pack("month-end-ready", tmp_path / "pack")
    prior = dest / "prior_trial_balance.csv"
    prior.write_text(
        prior.read_text(encoding="utf-8").replace("2026-05-31", "2026-07-31"),
        encoding="utf-8",
    )
    pack = review_pack(profile="month_end", pack_dir=dest)
    assert pack.status == "BLOCKED"
    assert any(item.code == FINDING_PERIOD_ORDER for item in pack.findings)


def test_acknowledgement_before_period_end_exits_one(tmp_path: Path) -> None:
    note = tmp_path / "note.json"
    note.write_text(
        json.dumps(
            {
                "reviewer_initials": "RD",
                "reviewed_on": "2026-01-01",
                "comment": "Dated before the period ended.",
            }
        ),
        encoding="utf-8",
    )
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(EXAMPLES / "bas-ready"),
                "--output",
                str(tmp_path / "out"),
                "--review-note",
                str(note),
            ]
        )
        == 1
    )
    assert not (tmp_path / "out" / "readiness-pack.json").exists()


def test_output_collision_with_review_note_exits_one(tmp_path: Path) -> None:
    output = tmp_path / "out"
    output.mkdir()
    collision = output / "readiness-pack.json"
    collision.write_text("{}", encoding="utf-8")
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(EXAMPLES / "bas-ready"),
                "--output",
                str(output),
                "--review-note",
                str(collision),
            ]
        )
        == 1
    )


def test_acknowledgement_never_flips_blocked(tmp_path: Path) -> None:
    pack = review_pack(
        profile="bas",
        pack_dir=EXAMPLES / "bas-blocked",
        acknowledgement_path=EXAMPLES / "review_note.json",
    )
    assert pack.status == "BLOCKED"
    assert pack.acknowledgement is not None
    assert pack.tieout_tolerance == Decimal("0.01")


def test_exception_tieout_is_not_ready(tmp_path: Path) -> None:
    from reviewready.engine import _apply_year_end_tieout, _overall
    from reviewready.models import TieOutRow
    findings = []
    _apply_year_end_tieout({"tie_out_matrix": [TieOutRow("Cash", Decimal("10"), "WP1", "cash.csv", "EXCEPTION")]}, findings)
    assert _overall(findings) == "NOT_READY"
    assert any(item.code == "TIEOUT_BREAK" for item in findings)


def test_preparer_cannot_acknowledge_own_pack(tmp_path: Path) -> None:
    import pytest
    from reviewready.errors import GateInputError
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    initials = json.loads((dest / "self_review.json").read_text())["preparer_initials"]
    note = tmp_path / "note.json"
    note.write_text(json.dumps({"reviewer_initials": " " + initials.lower() + " ", "reviewed_on": "2026-07-01", "comment": "Fabricated self-review."}))
    with pytest.raises(GateInputError, match="preparer"):
        review_pack(profile="bas", pack_dir=dest, acknowledgement_path=note)


def test_unreadable_pack_directory_returns_input_error(tmp_path: Path, monkeypatch, capsys) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    original = Path.iterdir
    def iterdir(path):
        if path == dest:
            raise PermissionError("fabricated denial")
        return original(path)
    monkeypatch.setattr(Path, "iterdir", iterdir)
    assert main(["gate", "--profile", "bas", "--pack", str(dest), "--output", str(tmp_path / "out")]) == 1
    assert "input error" in capsys.readouterr().err


@pytest.mark.parametrize(
    ("example", "profile", "fixture_date"),
    [
        ("bas-ready", "bas", "2026-03-31"),
        ("month-end-ready", "month_end", "2026-06-30"),
        ("year-end-ready", "year_end", "2026-06-30"),
    ],
)
@pytest.mark.parametrize("wrong_date", ["2025-12-31", "2026-12-31"])
def test_trial_balance_off_declared_period_is_blocked(
    tmp_path: Path, example: str, profile: str, fixture_date: str, wrong_date: str
) -> None:
    dest = copy_example_pack(example, tmp_path / "pack")
    tb = dest / "trial_balance.csv"
    tb.write_text(tb.read_text(encoding="utf-8").replace(fixture_date, wrong_date), encoding="utf-8")
    pack = review_pack(profile=profile, pack_dir=dest)
    assert pack.status == "BLOCKED"
    binding = [
        item for item in pack.findings
        if item.code == FINDING_PERIOD_ORDER and item.slot == "trial_balance"
    ]
    assert len(binding) == 1
    assert wrong_date in binding[0].reason and fixture_date in binding[0].reason


def test_declared_period_off_trial_balance_is_blocked(tmp_path: Path) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    payload = json.loads((dest / "self_review.json").read_text(encoding="utf-8"))
    payload["period_end"] = "2026-06-30"
    payload["prepared_on"] = "2026-07-10"
    (dest / "self_review.json").write_text(json.dumps(payload), encoding="utf-8")
    pack = review_pack(profile="bas", pack_dir=dest)
    assert pack.status == "BLOCKED"
    assert pack.period_end == "2026-06-30"
    assert any(
        item.code == FINDING_PERIOD_ORDER and item.slot == "trial_balance"
        for item in pack.findings
    )


def test_matching_trial_balance_date_stays_ready() -> None:
    for example, profile in (("bas-ready", "bas"), ("month-end-ready", "month_end"), ("year-end-ready", "year_end")):
        assert review_pack(profile=profile, pack_dir=EXAMPLES / example).status == "READY"


def test_empty_optional_artefact_is_a_finding_and_a_control_that_did_not_run(
    tmp_path: Path,
) -> None:
    """A zero-byte file used to record a digest and raise nothing.

    The pack then proved it had read an empty bank reconciliation and still
    reported READY. An empty file is not evidence, so it is named twice: as a
    finding about the file, and as the control it stopped from running.
    """
    dest = copy_example_pack("month-end-ready", tmp_path / "pack")
    (dest / "bank_rec.csv").write_bytes(b"")
    pack = review_pack(profile="month_end", pack_dir=dest)
    assert pack.status == "NOT_READY"
    assert any(
        item.code == FINDING_EMPTY_ARTEFACT and item.slot == "bank_rec"
        for item in pack.findings
    )
    assert [control.slot for control in pack.controls_not_run] == [
        "bank_rec",
        "prior_findings",
    ]
    assert any("empty" in control.reason for control in pack.controls_not_run)
    # The digest is still recorded: the file was read, and saying so is how a
    # reviewer knows which bytes the finding is about.
    assert any(item.slot == "bank_rec" for item in pack.source_evidence)


def test_a_ready_month_end_pack_names_the_controls_that_did_not_run(
    tmp_path: Path,
) -> None:
    """bank_rec is optional, so a month_end pack with no bank reconciliation in
    it is READY. The pack and the summary now say which controls that covers."""
    dest = copy_example_pack("month-end-ready", tmp_path / "pack")
    (dest / "bank_rec.csv").unlink()
    pack = review_pack(profile="month_end", pack_dir=dest)
    assert pack.status == "READY"
    assert pack.findings == ()
    assert [
        (control.slot, control.filename, control.reason)
        for control in pack.controls_not_run
    ] == [
        ("bank_rec", "bank_rec.csv", "no file was supplied"),
        ("prior_findings", "prior_findings.csv", "no file was supplied"),
    ]

    output = tmp_path / "out"
    assert main([
        "gate", "--profile", "month_end", "--pack", str(dest), "--output", str(output)
    ]) == 0
    summary = (output / "readiness-summary.md").read_text(encoding="utf-8")
    assert "- Controls not run: 2." in summary
    assert "## Controls not run" in summary
    assert "- `bank_rec` (`bank_rec.csv`): no file was supplied" in summary
    document = json.loads((output / "readiness-pack.json").read_text(encoding="utf-8"))
    assert document["overall_status"] == "READY"
    assert document["controls_not_run"][0] == {
        "slot": "bank_rec",
        "filename": "bank_rec.csv",
        "reason": "no file was supplied",
    }
    # The 3 artefacts still verify against each other with the new section in them.
    assert main(["view", "--pack-dir", str(output)]) == 0


def test_a_pack_with_every_control_run_says_so(tmp_path: Path) -> None:
    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    (dest / "prior_findings.csv").write_text(
        "FindingCode,Slot,Status\n", encoding="utf-8"
    )
    pack = review_pack(profile="bas", pack_dir=dest)
    assert pack.status == "READY"
    assert pack.controls_not_run == ()
    output = tmp_path / "out"
    assert main([
        "gate", "--profile", "bas", "--pack", str(dest), "--output", str(output)
    ]) == 0
    summary = (output / "readiness-summary.md").read_text(encoding="utf-8")
    assert "- Controls not run: 0." in summary
    assert "Every control this engagement profile configures ran." in summary


def test_a_pack_written_without_control_coverage_still_verifies(tmp_path: Path) -> None:
    """Packs released up to v0.1.7 carry no controls_not_run member at all.

    `view` only reads, so requiring the member would have failed every pack
    already written. A pack that states no coverage renders with no bullet and no
    section, exactly as its writer produced it.
    """
    from dataclasses import replace

    from reviewready.report import write_review_pack

    dest = copy_example_pack("bas-ready", tmp_path / "pack")
    pack = review_pack(profile="bas", pack_dir=dest)
    output = tmp_path / "out"
    write_review_pack(replace(pack, controls_not_run=None), output)

    document = json.loads((output / "readiness-pack.json").read_text(encoding="utf-8"))
    assert "controls_not_run" not in document
    summary = (output / "readiness-summary.md").read_text(encoding="utf-8")
    assert "Controls not run" not in summary
    assert main(["view", "--pack-dir", str(output)]) == 0
