from __future__ import annotations

from decimal import Decimal
from pathlib import Path

from tests.support import EXAMPLES
from reviewready.cli import main
from reviewready.engine import review_pack
from reviewready.models import FINDING_MISSING_ARTEFACT, FINDING_TB_UNBALANCED
from reviewready.viewer import render_review_sheet


def test_bas_ready_pack_is_ready() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-ready")
    assert pack.status == "READY"
    assert pack.findings == ()
    assert pack.period_end == "2026-03-31"
    assert pack.preparer_initials == "AB"


def test_bas_not_ready_missing_gst_and_repeat() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-not-ready")
    assert pack.status == "NOT_READY"
    codes = {item.code for item in pack.findings}
    assert FINDING_MISSING_ARTEFACT in codes
    repeats = [item for item in pack.findings if item.repeat]
    assert repeats
    assert any(item.slot == "gst_control_gl" and item.repeat for item in pack.findings)


def test_bas_blocked_unbalanced_trial_balance() -> None:
    pack = review_pack(profile="bas", pack_dir=EXAMPLES / "bas-blocked")
    assert pack.status == "BLOCKED"
    assert any(item.code == FINDING_TB_UNBALANCED for item in pack.findings)


def test_month_end_and_year_end_ready() -> None:
    month = review_pack(profile="month_end", pack_dir=EXAMPLES / "month-end-ready")
    year = review_pack(profile="year_end", pack_dir=EXAMPLES / "year-end-ready")
    assert month.status == "READY"
    assert year.status == "READY"


def test_acknowledgement_does_not_change_status() -> None:
    pack = review_pack(
        profile="bas",
        pack_dir=EXAMPLES / "bas-not-ready",
        acknowledgement_path=EXAMPLES / "review_note.json",
    )
    assert pack.status == "NOT_READY"
    assert pack.acknowledgement is not None
    assert pack.acknowledgement.reviewer_initials == "RD"


def test_gst_tieout_break(tmp_path: Path) -> None:
    source = EXAMPLES / "bas-ready"
    pack_dir = tmp_path / "pack"
    pack_dir.mkdir()
    for name in (
        "trial_balance.csv",
        "activity_statement.csv",
        "open_items.csv",
        "self_review.json",
    ):
        (pack_dir / name).write_bytes((source / name).read_bytes())
    (pack_dir / "gst_control_gl.csv").write_text(
        "Date,AccountID,AccountName,Debit,Credit,Description\n"
        "2026-01-15,820,GST Payable,0.00,1.00,Wrong amount\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="bas", pack_dir=pack_dir, tieout_tolerance=Decimal("0.01"))
    assert pack.status == "NOT_READY"
    assert any(item.code == "TIEOUT_BREAK" for item in pack.findings)


def test_cli_ready_and_view(tmp_path: Path) -> None:
    output = tmp_path / "out"
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
            ]
        )
        == 0
    )
    assert (output / "readiness-pack.json").is_file()
    assert main(["view", "--pack-dir", str(output)]) == 0
    sheet, digests = render_review_sheet(output)
    assert "**Overall status: READY**" in sheet
    assert "readiness-pack.json" in digests


def test_cli_not_ready_exit_two(tmp_path: Path) -> None:
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(EXAMPLES / "bas-not-ready"),
                "--output",
                str(tmp_path / "out"),
            ]
        )
        == 2
    )


def test_cli_malformed_headers_exit_one(tmp_path: Path) -> None:
    pack_dir = tmp_path / "bad"
    pack_dir.mkdir()
    for name in ("activity_statement.csv", "gst_control_gl.csv", "open_items.csv", "self_review.json"):
        (pack_dir / name).write_bytes((EXAMPLES / "bas-ready" / name).read_bytes())
    (pack_dir / "trial_balance.csv").write_text("Nope\n1\n", encoding="utf-8")
    assert (
        main(
            [
                "gate",
                "--profile",
                "bas",
                "--pack",
                str(pack_dir),
                "--output",
                str(tmp_path / "out"),
            ]
        )
        == 1
    )


def test_unsupported_tie_out(tmp_path: Path) -> None:
    source = EXAMPLES / "year-end-ready"
    pack_dir = tmp_path / "pack"
    pack_dir.mkdir()
    for name in (
        "trial_balance.csv",
        "prior_trial_balance.csv",
        "open_items.csv",
        "self_review.json",
    ):
        (pack_dir / name).write_bytes((source / name).read_bytes())
    (pack_dir / "tie_out_matrix.csv").write_text(
        "StatementLine,StatementAmount,WorkpaperRef,SourceFile,Status\n"
        "Cash,120000.00,,,UNSUPPORTED\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="year_end", pack_dir=pack_dir)
    assert pack.status == "NOT_READY"
    assert any(item.code == "TIEOUT_UNSUPPORTED" for item in pack.findings)


def test_bank_rec_break(tmp_path: Path) -> None:
    source = EXAMPLES / "month-end-ready"
    pack_dir = tmp_path / "pack"
    pack_dir.mkdir()
    for name in (
        "trial_balance.csv",
        "prior_trial_balance.csv",
        "open_items.csv",
        "self_review.json",
    ):
        (pack_dir / name).write_bytes((source / name).read_bytes())
    (pack_dir / "bank_rec.csv").write_text(
        "AccountID,StatementBalance,GLBalance\n100,120000.00,119000.00\n",
        encoding="utf-8",
    )
    pack = review_pack(profile="month_end", pack_dir=pack_dir)
    assert pack.status == "NOT_READY"
    assert any(item.code == "BANK_REC_BREAK" for item in pack.findings)


def test_usage_error_is_exit_one() -> None:
    assert main([]) == 1
