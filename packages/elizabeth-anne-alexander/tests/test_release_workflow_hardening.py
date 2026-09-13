"""Contracts for the active namespaced release boundary."""

from pathlib import Path

import pytest

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release-elizabeth-anne-alexander.yml"


def _workflow_text() -> str:
    if not (REPOSITORY_ROOT / ".git").exists() and not (REPOSITORY_ROOT / "IMPORTS.md").is_file():
        pytest.skip("release workflow is not included in source distributions")
    return WORKFLOW.read_text(encoding="utf-8")


def test_normal_release_is_triggered_only_by_namespaced_tags() -> None:
    workflow = _workflow_text()

    assert 'tags:\n      - "elizabeth-anne-alexander/v*"' in workflow
    release_job = workflow.split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]
    assert "if: github.event_name == 'push'" in release_job
    assert "inputs.tag" not in workflow
    assert "group: release-${{ github.repository }}-${{ github.ref_name }}" in workflow
    assert "cancel-in-progress: false" in workflow


def test_release_uses_the_hardened_shared_policy_contract() -> None:
    workflow = _workflow_text()
    release_job = workflow.split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]

    assert (
        "uses: ryanduguid/release-policy/.github/workflows/release-python.yml@"
        "fcf25e532e9eb60056ae6e5c819cf3125c4f4b91"
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
    pypi_job = workflow.split("  pypi:\n", 1)[1].split("\n  recover-pypi-0-2-3:", 1)[0]

    assert "needs: release" in pypi_job
    assert "name: pypi-elizabeth-anne-alexander" in pypi_job
    assert "id-token: write" in pypi_job
    assert "name: dist-${{ needs.release.outputs.stem }}-${{ needs.release.outputs.version }}" in pypi_job
    assert "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c" in pypi_job
    assert "pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33" in pypi_job
    assert "python -m build" not in pypi_job


def test_manual_recovery_is_bound_to_the_original_verified_release() -> None:
    workflow = _workflow_text()
    recovery_job = workflow.split("  recover-pypi-0-2-3:\n", 1)[1]

    assert "workflow_dispatch:\n  push:" in workflow
    assert (
        "if: github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'"
    ) in recovery_job
    assert "inputs." not in recovery_job
    assert "tag=elizabeth-anne-alexander/v0.2.3" in recovery_job
    assert "commit=874e92d85458e87ebf726b1fc35ca3c333c88934" in recovery_job
    assert "policy=fcf25e532e9eb60056ae6e5c819cf3125c4f4b91" in recovery_job
    assert ".immutable == true" in recovery_job
    assert 'gh release verify "$tag"' in recovery_job
    assert 'gh release verify-asset "$tag" "proof/$asset"' in recovery_job
    assert '--source-digest "$commit" --source-ref "refs/tags/$tag"' in recovery_job
    assert "--signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml" in recovery_job
    assert '--signer-digest "$policy"' in recovery_job
    assert "--predicate-type https://spdx.dev/Document/v2.3" in recovery_job
    assert "sha256sum --check SHA256SUMS" in recovery_job
    assert "name: pypi-elizabeth-anne-alexander" in recovery_job
    assert "pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33" in recovery_job
    assert "python -m build" not in recovery_job
    assert "contents: write" not in recovery_job
    assert recovery_job.index("sha256sum --check") < recovery_job.index("pypa/gh-action-pypi-publish@")
