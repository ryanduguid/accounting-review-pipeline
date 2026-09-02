from __future__ import annotations

import json
from decimal import Decimal
from pathlib import Path

from tests.support import EXAMPLES, copy_example_pack
from reviewready.cli import main
from reviewready.engine import review_pack
from reviewready.models import (
    FINDING_EMPTY_ARTEFACT,
    FINDING_OPEN_ITEM_INCOMPLETE,
    FINDING_PERIOD_ORDER,
    FINDING_SELF_REVIEW_INCOMPLETE,
)


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
