from __future__ import annotations

import subprocess
import sys
import tarfile
from pathlib import Path

from tests.support import ROOT


def test_source_distribution_contains_manager_review_evidence(tmp_path: Path) -> None:
    subprocess.run(
        [sys.executable, "-m", "build", "--sdist", "--outdir", str(tmp_path)],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    archives = list(tmp_path.glob("*.tar.gz"))
    assert len(archives) == 1

    expected_rooted_members = {
        "review_ready_gate-0.1.2/CITATION.cff",
        "review_ready_gate-0.1.2/evaluation/manager_review_gate/README.md",
        "review_ready_gate-0.1.2/evaluation/manager_review_gate/expected_results.json",
    }
    expected_suffixes = {
        member.removeprefix("review_ready_gate-0.1.2/")
        for member in expected_rooted_members
    }
    with tarfile.open(archives[0], "r:gz") as archive:
        suffixes = [member.partition("/")[2] for member in archive.getnames()]

    counts = {suffix: suffixes.count(suffix) for suffix in expected_suffixes}
    assert counts == {suffix: 1 for suffix in expected_suffixes}
