from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
import urllib.request
from pathlib import Path

import pytest
from closecontrol.cli import main

ROOT = Path(__file__).resolve().parents[1]
GUIDE = ROOT / "docs" / "reviewer-quick-start.md"
TEXT = GUIDE.read_text(encoding="utf-8")
INPUTS = ("current_trial_balance.csv", "prior_trial_balance.csv", "account_mapping.csv", "subledger_balances.csv")
LAUNCHER = "uvx --from monthly-close-control-plane==0.1.5 close-control"
LIVE = "MONTHLY_CLOSE_QUICK_START_LIVE"


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
    # test_published_quick_start_end_to_end checks the release and downloads.
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


@pytest.mark.skipif(os.environ.get(LIVE) != "1", reason=f"set {LIVE}=1 to download and run the published release")
def test_published_quick_start_end_to_end(tmp_path: Path) -> None:
    # Runs the guide as a reviewer would: the pinned downloads and the exact
    # uvx commands against published 0.1.5. Needs network access and uv.
    for name in INPUTS:
        urllib.request.urlretrieve(f"{_base_url()}/{name}", tmp_path / name)

    def run(subcommand: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(shlex.split(_line(subcommand)), cwd=tmp_path, capture_output=True, text=True)

    review = run("review")
    assert review.returncode == 2, review.stderr
    assert review.stdout.splitlines()[:1] == [_printed_line()], (review.stdout, review.stderr)
    assert run("view").returncode == 0

    summary = tmp_path / "close-pack" / "close-summary.md"
    summary.write_text(summary.read_text(encoding="utf-8").replace("REVIEW", "PASS", 1), encoding="utf-8")
    edited = run("view")
    assert edited.returncode == 1
    assert "verification failed" in edited.stderr
