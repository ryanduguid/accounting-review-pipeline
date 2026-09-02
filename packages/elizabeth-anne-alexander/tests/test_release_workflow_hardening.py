"""Contracts for the active namespaced release boundary."""

from pathlib import Path

import pytest


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release-elizabeth-anne-alexander.yml"


def _workflow_text() -> str:
    if not (REPOSITORY_ROOT / ".git").exists() and not (REPOSITORY_ROOT / "IMPORTS.md").is_file():
        pytest.skip("release workflow is not included in source distributions")
    return WORKFLOW.read_text(encoding="utf-8")


def test_release_is_triggered_only_by_namespaced_tags() -> None:
    workflow = _workflow_text()

    assert 'tags:\n      - "elizabeth-anne-alexander/v*"' in workflow
    assert "workflow_dispatch:" not in workflow
    assert "inputs.tag" not in workflow
    assert "group: release-${{ github.repository }}-${{ github.ref_name }}" in workflow
    assert "cancel-in-progress: false" in workflow


def test_release_uses_the_hardened_shared_policy_contract() -> None:
    workflow = _workflow_text()
    release_job = workflow.split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]

    assert (
        "uses: ryanduguid/release-policy/.github/workflows/release-python.yml@"
        "3ff09b654a17b9a3b55548e25e6108ee582b00c4"
    ) in release_job
    assert "actions: read" in release_job
    assert "source-directory: packages/elizabeth-anne-alexander" in release_job
    assert "tag-prefix: elizabeth-anne-alexander" in release_job
    assert "upload-dist-artifact: true" in release_job
    assert "version-command:" not in release_job
    assert "version-parser: python-literal" in release_job
    assert "version-file: elizabeth_anne_alexander/version.py" in release_job


def test_pypi_uses_only_the_exact_attested_distribution() -> None:
    workflow = _workflow_text()
    pypi_job = workflow.split("  pypi:\n", 1)[1]

    assert "needs: release" in pypi_job
    assert "name: pypi-elizabeth-anne-alexander" in pypi_job
    assert "id-token: write" in pypi_job
    assert "name: dist-${{ needs.release.outputs.stem }}-${{ needs.release.outputs.version }}" in pypi_job
    assert "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c" in pypi_job
    assert "pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33" in pypi_job
    assert "python -m build" not in pypi_job
