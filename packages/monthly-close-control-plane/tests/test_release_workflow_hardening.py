"""Contracts for the manually dispatched release backfill boundary."""

import re
from pathlib import Path

import pytest


REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release-monthly-close-control-plane.yml"
TAG_PATTERN = re.compile(
    r"monthly-close-control-plane/v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
)


def _workflow_text() -> str:
    if not (REPOSITORY_ROOT / ".git").exists() and not (REPOSITORY_ROOT / "IMPORTS.md").is_file():
        pytest.skip("release workflow is not included in source distributions")
    return WORKFLOW.read_text(encoding="utf-8")


def test_release_backfill_treats_dispatch_tag_as_validated_data() -> None:
    workflow = _workflow_text()

    assert "TAG: ${{ inputs.tag }}" in workflow
    assert (
        r'if [[ ! "$TAG" =~ '
        r'^monthly-close-control-plane/v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then'
    ) in workflow
    assert 'gh release download "$TAG"' in workflow
    assert 'gh release download "${{ inputs.tag }}"' not in workflow
    assert '--repo "$GITHUB_REPOSITORY"' in workflow
    assert '--repo "${{ github.repository }}"' not in workflow
    assert (
        "group: release-${{ github.repository }}-"
        "${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}"
    ) in workflow
    assert "cancel-in-progress: false" in workflow


def test_release_tag_validator_rejects_shell_shaped_and_malformed_values() -> None:
    for tag in (
        "1.2.3", "v1.2.3", "review-ready-gate/v1.2.3",
        "monthly-close-control-plane/v1.2", "monthly-close-control-plane/v1.2.3/extra",
        'monthly-close-control-plane/v1.2.3"; touch PWNED; #',
    ):
        assert TAG_PATTERN.fullmatch(tag) is None

    for tag in ("monthly-close-control-plane/v0.1.3", "monthly-close-control-plane/v12.0.103"):
        assert TAG_PATTERN.fullmatch(tag) is not None


@pytest.mark.parametrize("version", ("v01.2.3", "v1.02.3", "v1.2.003"))
def test_release_tag_validator_rejects_leading_zero_numeric_identifiers(version: str) -> None:
    assert TAG_PATTERN.fullmatch(f"monthly-close-control-plane/{version}") is None


def test_release_uses_the_hardened_shared_policy_contract() -> None:
    workflow = _workflow_text()
    release_job = workflow.split("  release:\n", 1)[1].split("\n  pypi:", 1)[0]

    assert (
        "uses: ryanduguid/release-policy/.github/workflows/release-python.yml@"
        "3ff09b654a17b9a3b55548e25e6108ee582b00c4"
    ) in release_job
    assert "actions: read" in release_job
    assert "source-directory: packages/monthly-close-control-plane" in release_job
    assert "tag-prefix: monthly-close-control-plane" in release_job
    assert "version-command:" not in release_job
    assert "version-parser:" not in release_job
    assert "version-file:" not in release_job
