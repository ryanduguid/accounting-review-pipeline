"""Contracts for the blocked native release boundary.

review-ready-gate 1.0.0 is a Rust binary with no native publisher. The
release workflow deliberately refuses every publication until a native
release policy is configured and approved, so these tests pin the exact
permitted shape: reachable on namespaced tags and manual dispatch only,
read-only permissions, a single refusing job with no action, reusable
workflow, upload or success path. Restoring an active publisher requires
replacing this file and these tests together.
"""

import re
from pathlib import Path

import pytest

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = REPOSITORY_ROOT / ".github" / "workflows" / "release-review-ready-gate.yml"


def _workflow_text() -> str:
    if not (REPOSITORY_ROOT / ".git").exists() and not (REPOSITORY_ROOT / "IMPORTS.md").is_file():
        pytest.skip("release workflow is not included in source distributions")
    return WORKFLOW.read_text(encoding="utf-8")


def test_release_triggers_exactly_namespaced_tags_and_manual_dispatch() -> None:
    workflow = _workflow_text()

    assert re.search(
        r"(?ms)^on:\n  push:\n    tags: \['review-ready-gate/v\*'\]\n  workflow_dispatch:\n",
        workflow,
    )
    # Exactly the two sanctioned events; nothing else may trigger publication attempts.
    assert workflow.count("push:") == 1
    assert workflow.count("workflow_dispatch:") == 1
    for forbidden in ("pull_request", "schedule", "inputs."):
        assert forbidden not in workflow


def test_release_permissions_stay_read_only() -> None:
    workflow = _workflow_text()

    permissions = workflow.split("permissions:", 1)[1].split("jobs:", 1)[0]
    assert permissions.strip() == "contents: read"
    assert "id-token" not in workflow
    assert "write" not in workflow


def test_release_is_one_refusing_job() -> None:
    workflow = _workflow_text()
    jobs = workflow.split("jobs:", 1)[1]

    assert re.search(r"(?m)^  release-blocked:\n", jobs)
    # No release, pypi or other job key may exist alongside the refusal.
    assert not re.search(r"(?m)^  (?!release-blocked:)[a-z][a-z-]*:\n", jobs)
    assert "runs-on: ubuntu-latest" in jobs
    assert "timeout-minutes: 2" in jobs
    # No reusable workflow or action call anywhere in the job.
    assert "uses:" not in jobs


def test_release_refuses_publication_unconditionally() -> None:
    workflow = _workflow_text()
    steps = workflow.split("steps:", 1)[1]

    assert steps.count("- name:") == 1
    assert "Refuse publication until the native release policy is configured" in steps
    assert "::error::" in steps
    assert "exit 1" in steps
    # The refusal must not be conditional or bypassable.
    assert "if:" not in steps
    assert "continue-on-error" not in workflow
