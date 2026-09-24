from __future__ import annotations

import json
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
        "review_ready_gate-0.1.5/CITATION.cff",
        "review_ready_gate-0.1.5/evaluation/manager_review_gate/README.md",
        "review_ready_gate-0.1.5/evaluation/manager_review_gate/expected_results.json",
        "review_ready_gate-0.1.5/evaluation/missing_evidence/expected_results.json",
        "review_ready_gate-0.1.5/evaluation/missing_evidence/packs/empty_bank_reconciliation/bank_rec.csv",
        "review_ready_gate-0.1.5/evaluation/missing_evidence/packs/wrong_period/self_review.json",
    }
    expected_suffixes = {
        member.removeprefix("review_ready_gate-0.1.5/")
        for member in expected_rooted_members
    }
    with tarfile.open(archives[0], "r:gz") as archive:
        suffixes = [member.partition("/")[2] for member in archive.getnames()]

    counts = {suffix: suffixes.count(suffix) for suffix in expected_suffixes}
    assert counts == {suffix: 1 for suffix in expected_suffixes}

    # Only the declared fabricated fixtures may ship as evaluation CSVs.
    contract = json.loads(
        (ROOT / "evaluation" / "missing_evidence" / "expected_results.json").read_text(encoding="utf-8")
    )
    base = {path.name for path in (ROOT / contract["base_fixture"]).glob("*.csv")}
    declared = {
        f"{scenario['fixture']}/{name}"
        for scenario in contract["scenarios"]
        for name in base - set(scenario["change"].get("removed", []))
    }
    packaged = {s for s in suffixes if s.startswith("evaluation/") and s.lower().endswith(".csv")}
    assert packaged == declared
