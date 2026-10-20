"""Contracts for the active namespaced release boundary."""

import re
from pathlib import Path

import pytest

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release-review-ready-gate.yml"


def _workflow_text() -> str:
    if not (REPOSITORY_ROOT / ".git").exists() and not (REPOSITORY_ROOT / "IMPORTS.md").is_file():
        pytest.skip("release workflow is not included in source distributions")
    return WORKFLOW.read_text(encoding="utf-8")


def test_release_is_triggered_only_by_namespaced_tags() -> None:
    workflow = _workflow_text()

    assert 'tags:\n      - "review-ready-gate/v*"' in workflow
    assert "workflow_dispatch:" not in workflow
    assert "inputs.tag" not in workflow
    assert "group: release-${{ github.repository }}-${{ github.ref_name }}" in workflow
    assert "cancel-in-progress: false" in workflow


def test_release_uses_the_hardened_shared_policy_contract() -> None:
    workflow = _workflow_text()
    release_job = workflow.split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]

    # The pin itself is the callers' to set; tests/test_release_pins.py holds them together.
    assert re.search(
        r"(?m)^    uses: ryanduguid/release-policy/\.github/workflows/release-python\.yml@[0-9a-f]{40}$",
        release_job,
    )
    assert "actions: read" in release_job
    assert "source-directory: packages/review-ready-gate" in release_job
    assert "tag-prefix: review-ready-gate" in release_job
    assert "upload-dist-artifact: true" in release_job
    assert "version-command:" not in release_job
    assert "version-parser:" not in release_job
    assert "version-file:" not in release_job


def test_pypi_uses_only_the_exact_attested_distribution() -> None:
    workflow = _workflow_text()
    pypi_job = workflow.split("  pypi:\n", 1)[1]

    assert "needs: release" in pypi_job
    assert "name: pypi-review-ready-gate" in pypi_job
    assert "id-token: write" in pypi_job
    assert "artifact-ids: ${{ needs.release.outputs.dist-id }}" in pypi_job
    assert "python policy/scripts/python_release.py verify-candidate" in pypi_job
    # Full commit SHAs, so Dependabot can advance them without editing this test.
    assert re.search(r"actions/download-artifact@[0-9a-f]{40} # v\d", pypi_job)
    assert re.search(r"pypa/gh-action-pypi-publish@[0-9a-f]{40} # v\d", pypi_job)
    assert "python -m build" not in pypi_job
