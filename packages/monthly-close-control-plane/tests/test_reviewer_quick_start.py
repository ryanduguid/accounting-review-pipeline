from __future__ import annotations

import re
import shlex
import shutil
from pathlib import Path

import pytest
from closecontrol.cli import main

ROOT = Path(__file__).resolve().parents[1]
GUIDE = ROOT / "docs" / "reviewer-quick-start.md"
TEXT = GUIDE.read_text(encoding="utf-8")
INPUTS = ("current_trial_balance.csv", "prior_trial_balance.csv", "account_mapping.csv", "subledger_balances.csv")


def _command(subcommand: str) -> list[str]:
    line = next(
        line for line in TEXT.splitlines() if line.startswith(f"uvx --from monthly-close-control-plane==0.1.5 close-control {subcommand} ")
    )
    return shlex.split(line)[4:]


def test_both_download_blocks_fetch_the_same_inputs_from_one_commit() -> None:
    bases = re.findall(r"https://raw\.githubusercontent\.com/\S+?/examples", TEXT)
    assert len(bases) == 2 and len(set(bases)) == 1
    assert re.search(r"/accounting-review-pipeline/[0-9a-f]{40}/packages/", bases[0])
    for name in INPUTS:
        assert TEXT.count(f'"{name}"') == 1  # PowerShell list
        assert re.search(rf"for f in .*\b{re.escape(name)}\b", TEXT)  # bash list
        assert (ROOT / "examples" / name).is_file()


def test_the_review_command_prints_what_the_guide_says(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    # The guide runs the published release; this pins that the documented
    # command and counts still hold for the examples it downloads.
    for name in INPUTS:
        shutil.copyfile(ROOT / "examples" / name, tmp_path / name)
    monkeypatch.chdir(tmp_path)
    assert main(_command("review")) == 2
    printed = capsys.readouterr().out.splitlines()[0]
    assert f"It prints `{printed}`" in " ".join(TEXT.split())
    assert main(_command("view")) == 0

    summary = tmp_path / "close-pack" / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8").replace("REVIEW", "PASS", 1), encoding="utf-8")
    assert main(_command("view")) == 1
    assert "verification failed" in capsys.readouterr().err
