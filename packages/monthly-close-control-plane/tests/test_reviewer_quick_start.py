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
LAUNCHER = "uvx --from monthly-close-control-plane==0.1.5 close-control"


def _line(subcommand: str) -> str:
    return next(line for line in TEXT.splitlines() if line.startswith(f"{LAUNCHER} {subcommand} "))


def _base_url() -> str:
    bases = re.findall(r"https://raw\.githubusercontent\.com/\S+?/examples", TEXT)
    assert len(bases) == 2 and len(set(bases)) == 1
    return bases[0]


def _printed_line() -> str:
    return re.search(r"It prints `([^`]+)`", " ".join(TEXT.split())).group(1)


def test_both_download_blocks_fetch_the_same_inputs_from_one_commit() -> None:
    assert re.search(r"/accounting-review-pipeline/[0-9a-f]{40}/packages/", _base_url())
    for name in INPUTS:
        assert TEXT.count(f'"{name}"') == 1  # PowerShell list
        assert re.search(rf"for f in .*\b{re.escape(name)}\b", TEXT)  # bash list
        assert (ROOT / "examples" / name).is_file()


def test_the_checkout_matches_the_guide_on_its_own_examples(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    # Offline check of this checkout only: the documented arguments, run by the
    # source CLI on examples/, still give the printed line and view behaviour.
    # The reviewer-quick-start workflow runs the published release on the
    # pinned downloads; package tests stay offline.
    for name in INPUTS:
        shutil.copyfile(ROOT / "examples" / name, tmp_path / name)
    monkeypatch.chdir(tmp_path)
    assert main(shlex.split(_line("review"))[4:]) == 2
    assert capsys.readouterr().out.splitlines()[0] == _printed_line()
    assert main(shlex.split(_line("view"))[4:]) == 0

    summary = tmp_path / "close-pack" / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8").replace("REVIEW", "PASS", 1), encoding="utf-8")
    assert main(shlex.split(_line("view"))[4:]) == 1
    assert "verification failed" in capsys.readouterr().err

