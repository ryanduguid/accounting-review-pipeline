"""Contracts for the active namespaced release boundary."""

from pathlib import Path

import pytest

from test_workflow_examples import _load_workflow

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release-monthly-close-control-plane.yml"


def _workflow_text() -> str:
    if not (REPOSITORY_ROOT / ".git").exists() and not (REPOSITORY_ROOT / "IMPORTS.md").is_file():
        pytest.skip("release workflow is not included in source distributions")
    return WORKFLOW.read_text(encoding="utf-8")


def test_release_is_triggered_only_by_namespaced_tags() -> None:
    workflow = _workflow_text()

    assert _load_workflow(workflow)["on"] == {"push": {"tags": ["monthly-close-control-plane/v*"]}}
    assert "workflow_dispatch:" not in workflow
    assert "inputs.tag" not in workflow
    assert "group: release-${{ github.repository }}-${{ github.ref_name }}" in workflow
    assert "cancel-in-progress: false" in workflow


def test_release_uses_the_hardened_shared_policy_contract() -> None:
    workflow = _workflow_text()
    release_job = workflow.split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]

    assert (
        "uses: ryanduguid/release-policy/.github/workflows/release-python.yml@"
        "171aa487dbc0a8f437ed84407f0d506f814548c1"
    ) in release_job
    assert "actions: read" in release_job
    assert "source-directory: packages/monthly-close-control-plane" in release_job
    assert "tag-prefix: monthly-close-control-plane" in release_job
    assert "version-command:" not in release_job
    assert "version-parser:" not in release_job
    assert "version-file:" not in release_job
