from __future__ import annotations

import sys

if sys.version_info >= (3, 11):
    import tomllib
else:
    import tomli as tomllib

from tests.support import EXAMPLES, ROOT

WORKFLOW = EXAMPLES / "github-actions-readiness-check.yml"
REPOSITORY_NAME = "workpaper-review-gate"
PACKAGE_NAME = "review-ready-gate"
REPOSITORY_URL = f"https://github.com/ryanduguid/{REPOSITORY_NAME}"
HOMEPAGE_URL = "https://duguid.com.au/tools/workpaper-review-gate/"

EXPECTED_PINS = (
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
    "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97 # v7.0.0",
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1",
)


def test_copyable_workflow_discovery_is_not_vacuous() -> None:
    workflows = sorted({*EXAMPLES.rglob("*.yml"), *EXAMPLES.rglob("*.yaml")})
    assert workflows == [WORKFLOW]


def test_copyable_workflow_is_scheduled_and_fail_closed_on_blocked() -> None:
    text = WORKFLOW.read_text(encoding="utf-8")
    for pin in EXPECTED_PINS:
        assert pin in text
    assert "persist-credentials: false" in text
    assert "contents: read" in text
    assert "python -m pip install ." in text
    assert "--pack examples/bas-not-ready" in text
    assert 'if [ "$status" = "BLOCKED" ]' in text
    assert "NOT_READY" in text
    assert "READY still does not mean the file is approved" in text
    assert "fabricated or synthetic data" in text
    assert "Never commit a\n# client workpaper pack." in text


def test_release_guidance_is_repo_specific_and_durable() -> None:
    text = (ROOT / "RELEASING.md").read_text(encoding="utf-8")
    normalised = " ".join(text.split())
    assert f"repos/ryanduguid/{REPOSITORY_NAME}/immutable-releases" in text
    assert "Workflow filename | `release.yml`" in text
    assert "Environment name | `pypi`" in text
    assert 'wheel="review_ready_gate-${tag#v}-py3-none-any.whl"' in text
    assert "intend to publish." in text
    assert "This release uses version `0.1.2`." in normalised
    assert "published `v0.1.1` recovery" in normalised
    assert "intended to be" not in text

    contributing = " ".join(
        (ROOT / "CONTRIBUTING.md").read_text(encoding="utf-8").split()
    )
    assert (
        "For a first PyPI publication, complete the one-time `pypi` environment "
        "and trusted-publisher setup in that file before tagging."
    ) in contributing
    assert "no published PyPI project yet" not in contributing


def test_current_release_metadata_uses_immutable_documentation() -> None:
    metadata = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    assert metadata["project"]["version"] == "0.1.2"
    assert metadata["project"]["urls"]["Documentation"] == (
        f"{REPOSITORY_URL}/tree/v0.1.2/"
        "evaluation/manager_review_gate"
    )


def test_release_notes_heading_matches_release_policy_tag() -> None:
    metadata = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    heading = (ROOT / "RELEASE_NOTES.md").read_text(encoding="utf-8").splitlines()[0]
    assert heading == f"# v{metadata['project']['version']}"


def test_readme_development_uses_the_locked_uv_entrypoint() -> None:
    development = (ROOT / "README.md").read_text(encoding="utf-8").split("## Development", 1)[1]
    block = development.split("```bash", 1)[1].split("```", 1)[0]
    assert "uv run pytest" in block
    assert "uv lock --check" in block
    assert "python -m pytest" not in block


def test_citation_declares_the_current_release_identity() -> None:
    citation = (ROOT / "CITATION.cff").read_text(encoding="utf-8")
    assert 'title: "Workpaper Review Gate"' in citation
    assert "version: 0.1.2" in citation
    assert 'family-names: "Duguid"' in citation
    assert 'given-names: "Ryan"' in citation
    assert "license: MIT" in citation
    assert f'repository-code: "{REPOSITORY_URL}"' in citation


def test_project_identity_preserves_package_and_command_compatibility() -> None:
    metadata = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))

    assert metadata["project"]["name"] == PACKAGE_NAME
    assert metadata["project"]["scripts"]["review-ready"] == "reviewready.cli:main"
    assert metadata["project"]["urls"]["Homepage"] == HOMEPAGE_URL
    assert metadata["project"]["urls"]["Repository"] == f"{REPOSITORY_URL}.git"


def test_release_attestation_commands_bind_the_exact_signing_identity() -> None:
    text = (ROOT / "RELEASING.md").read_text(encoding="utf-8")
    attestation_commands = []
    for bash_block in text.split("```bash")[1:]:
        lines = iter(bash_block.split("```", 1)[0].splitlines())
        for line in lines:
            if not line.startswith("gh attestation verify "):
                continue

            command_lines = [line]
            while command_lines[-1].endswith("\\"):
                continuation = next(lines, None)
                assert continuation is not None
                command_lines.append(continuation)
            attestation_commands.append("\n".join(command_lines))

    assert len(attestation_commands) == 2
    for command in attestation_commands:
        assert "--signer-repo" not in command
        assert command.count('-R "$repo"') == 1
        assert command.count('--source-digest "$release_commit"') == 1
        assert command.count('--source-ref "refs/tags/$tag"') == 1
        assert command.count(
            "--signer-workflow "
            "ryanduguid/release-policy/.github/workflows/release-python.yml"
        ) == 1
        assert command.count(
            "--signer-digest 8b4de1ed339f1358b5f3e850b63412d8717d01da"
        ) == 1

    predicate_counts = [
        command.count("--predicate-type https://spdx.dev/Document/v2.3")
        for command in attestation_commands
    ]
    assert sorted(predicate_counts) == [0, 1]


def test_discovery_record_matches_the_approved_remote_metadata() -> None:
    discovery = (ROOT / "docs" / "DISCOVERY.md").read_text(encoding="utf-8")
    assert discovery == (
        "# GitHub discovery metadata\n\n"
        "Description: Stop incomplete workpapers reaching manager review. "
        "Deterministic readiness gate for Australian public-practice packs. Not advice.\n\n"
        f"Homepage: {HOMEPAGE_URL}\n\n"
        "Topics:\n\n"
        "- accounting\n"
        "- accounting-controls\n"
        "- australia\n"
        "- bas\n"
        "- cli\n"
        "- month-end\n"
        "- public-practice\n"
        "- python\n"
        "- quality-control\n"
        "- review\n"
        "- review-workflow\n"
        "- workpaper-review\n"
        "- workpapers\n"
        "- year-end\n"
    )
