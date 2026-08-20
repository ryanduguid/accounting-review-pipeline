from __future__ import annotations

import re
from collections import Counter
from pathlib import Path

import pytest

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - exercised by the Python 3.10 CI job
    import tomli as tomllib


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
WORKFLOW = EXAMPLES / "github-actions-close-check.yml"

PACKAGE_NAME = "monthly-close-control-plane"
RELEASE_ASSET_SHA256 = "e4ca2bce708a3e28c8a6316eae68095848a116a04a99f24bc1d7325d92a449d9"

EXPECTED_EXTERNAL_PINS = {
    "actions/checkout": (
        "3d3c42e5aac5ba805825da76410c181273ba90b1",
        "v7.0.1",
    ),
    "actions/setup-python": (
        "5fda3b95a4ea91299a34e894583c3862153e4b97",
        "v7.0.0",
    ),
    "actions/upload-artifact": (
        "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        "v7.0.1",
    ),
}

USES_LINE = re.compile(
    r"^\s*(?:-\s*)?uses:\s*(?P<value>.*?)(?:\s+#\s*(?P<comment>.*?))?\s*$"
)
RUN_LINE = re.compile(r"^\s*(?:-\s*)?run:\s*(?P<command>.+?)\s*$")
PINNED_ACTION = re.compile(
    r"(?P<action>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*)"
    r"@(?P<sha>[0-9a-fA-F]{40})"
)
SEMANTIC_RELEASE = re.compile(r"v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?")
WHOLE_EXPRESSION = re.compile(r"\$\{\{.+\}\}")


def _unquote(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


def _install_command(project_version: str) -> str:
    requirement = (
        f"{PACKAGE_NAME} @ "
        "https://github.com/ryanduguid/monthly-close-control-plane/releases/download/"
        f"v{project_version}/monthly_close_control_plane-{project_version}-py3-none-any.whl"
        f"#sha256={RELEASE_ASSET_SHA256}"
    )
    return f'python -m pip install "{requirement}"'


def _active_uses(text: str) -> list[tuple[int, str, str | None]]:
    found: list[tuple[int, str, str | None]] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        match = USES_LINE.fullmatch(line)
        if match:
            found.append(
                (
                    line_number,
                    _unquote(match.group("value")),
                    match.group("comment"),
                )
            )
    return found


def _checkout_does_not_persist_credentials(text: str) -> bool:
    lines = text.splitlines()
    for index, line in enumerate(lines):
        match = USES_LINE.fullmatch(line)
        if not match or not _unquote(match.group("value")).startswith("actions/checkout@"):
            continue

        step_indent = len(line) - len(line.lstrip())
        block: list[str] = []
        for following in lines[index + 1 :]:
            stripped = following.lstrip()
            following_indent = len(following) - len(stripped)
            if stripped.startswith("-") and following_indent <= step_indent:
                break
            block.append(following)

        return any(
            re.fullmatch(r"\s*persist-credentials:\s*false\s*", following)
            for following in block
        )
    return False


def _has_read_only_top_level_permissions(text: str) -> bool:
    lines = text.splitlines()
    permission_blocks = [index for index, line in enumerate(lines) if line == "permissions:"]
    if len(permission_blocks) != 1:
        return False

    values: list[str] = []
    for line in lines[permission_blocks[0] + 1 :]:
        if line and not line.startswith((" ", "#")):
            break
        stripped = line.strip()
        if stripped and not stripped.startswith("#"):
            values.append(stripped)
    return values == ["contents: read"]


def _has_client_data_warning(text: str) -> bool:
    comment_text = " ".join(
        line.lstrip()[1:].strip()
        for line in text.splitlines()
        if line.lstrip().startswith("#")
    )
    return (
        "fabricated or synthetic data" in comment_text
        and "Never commit a client trial balance." in comment_text
    )


def _workflow_errors(text: str, *, project_version: str) -> list[str]:
    errors: list[str] = []
    actual_pins: list[tuple[str, str, str | None]] = []

    for line_number, value, comment in _active_uses(text):
        if value.startswith("./") or WHOLE_EXPRESSION.fullmatch(value):
            continue

        match = PINNED_ACTION.fullmatch(value)
        if not match:
            errors.append(
                f"line {line_number}: external uses value must end in an exact 40-character "
                f"commit SHA, got {value!r}"
            )
            continue

        action = match.group("action")
        sha = match.group("sha")
        if comment is None or not SEMANTIC_RELEASE.fullmatch(comment):
            errors.append(
                f"line {line_number}: {action} must retain a full semantic-version comment"
            )
        actual_pins.append((action, sha, comment))

    expected_pins = [
        (action, sha, release)
        for action, (sha, release) in EXPECTED_EXTERNAL_PINS.items()
    ]
    if Counter(actual_pins) != Counter(expected_pins):
        errors.append(
            "external action pins must exactly match the reviewed action/SHA/release set; "
            f"expected {expected_pins!r}, got {actual_pins!r}"
        )

    install_commands = []
    for line in text.splitlines():
        match = RUN_LINE.fullmatch(line)
        if match and "pip install" in match.group("command") and PACKAGE_NAME in match.group("command"):
            install_commands.append(match.group("command"))
    expected_install = _install_command(project_version)
    if install_commands != [expected_install]:
        errors.append(
            "the active package install must be the one reviewed immutable release-asset "
            f"reference; expected {expected_install!r}, got {install_commands!r}"
        )

    if not _has_read_only_top_level_permissions(text):
        errors.append("top-level workflow permissions must remain exactly contents: read")
    if not _checkout_does_not_persist_credentials(text):
        errors.append("checkout must set persist-credentials: false")
    if not _has_client_data_warning(text):
        errors.append("the fabricated-data and never-commit-client-data warning must remain")

    return errors


def _assert_workflow(text: str, *, project_version: str = "0.1.1") -> None:
    errors = _workflow_errors(text, project_version=project_version)
    assert not errors, "\n".join(errors)


def _secure_workflow() -> str:
    return f"""# Store only fabricated or synthetic data in the repository. Never commit a
# client trial balance.
permissions:
  contents: read
jobs:
  close-check:
    steps:
      - uses: actions/checkout@{EXPECTED_EXTERNAL_PINS['actions/checkout'][0]} # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/setup-python@{EXPECTED_EXTERNAL_PINS['actions/setup-python'][0]} # v7.0.0
      - name: Install close-control
        run: {_install_command('0.1.1')}
      - uses: actions/upload-artifact@{EXPECTED_EXTERNAL_PINS['actions/upload-artifact'][0]} # v7.0.1
"""


def _repository_versions() -> tuple[str, str, str]:
    project = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((ROOT / "uv.lock").read_text(encoding="utf-8"))
    root_packages = [
        package
        for package in lock["package"]
        if package["name"] == PACKAGE_NAME and package.get("source") == {"editable": "."}
    ]
    assert len(root_packages) == 1, "uv.lock must contain exactly one editable root package"

    release_heading = (ROOT / "RELEASE_NOTES.md").read_text(encoding="utf-8").splitlines()[0]
    release_match = re.fullmatch(r"# v(?P<version>\d+\.\d+\.\d+)", release_heading)
    assert release_match, "RELEASE_NOTES.md must start with a semantic version heading"
    return (
        project["project"]["version"],
        root_packages[0]["version"],
        release_match.group("version"),
    )


def test_copyable_workflow_discovery_is_not_vacuous() -> None:
    workflows = sorted({*EXAMPLES.rglob("*.yml"), *EXAMPLES.rglob("*.yaml")})

    assert workflows == [WORKFLOW]


def test_repository_version_owners_agree() -> None:
    project_version, lock_version, release_version = _repository_versions()

    assert project_version == lock_version == release_version == "0.1.1"


def test_committed_copyable_workflow_uses_only_reviewed_immutable_dependencies() -> None:
    project_version, _, _ = _repository_versions()

    _assert_workflow(WORKFLOW.read_text(encoding="utf-8"), project_version=project_version)


@pytest.mark.parametrize("action", EXPECTED_EXTERNAL_PINS)
@pytest.mark.parametrize("replacement_kind", ["tag", "main", "short-sha", "other-sha"])
def test_action_pin_mutations_are_rejected(action: str, replacement_kind: str) -> None:
    text = _secure_workflow()
    sha, release = EXPECTED_EXTERNAL_PINS[action]
    replacements = {
        "tag": release,
        "main": "main",
        "short-sha": sha[:-1],
        "other-sha": "0" * 40,
    }

    with pytest.raises(AssertionError):
        _assert_workflow(text.replace(f"{action}@{sha}", f"{action}@{replacements[replacement_kind]}"))


@pytest.mark.parametrize("action", EXPECTED_EXTERNAL_PINS)
@pytest.mark.parametrize("comment_mutation", ["remove", "alter"])
def test_action_release_comment_mutations_are_rejected(
    action: str, comment_mutation: str
) -> None:
    text = _secure_workflow()
    sha, release = EXPECTED_EXTERNAL_PINS[action]
    original = f"{action}@{sha} # {release}"
    replacement_comment = "" if comment_mutation == "remove" else f" # {release}-changed"
    replacement = f"{action}@{sha}{replacement_comment}"

    with pytest.raises(AssertionError):
        _assert_workflow(text.replace(original, replacement))


@pytest.mark.parametrize(
    ("old", "new"),
    [
        ("/download/v0.1.1/", "/download/v0.1.0/"),
        ("monthly_close_control_plane-0.1.1-py3", "monthly_close_control_plane-0.1.0-py3"),
        ("github.com/ryanduguid/monthly-close-control-plane", "github.com/example/monthly-close-control-plane"),
        (RELEASE_ASSET_SHA256, "0" + RELEASE_ASSET_SHA256[1:]),
    ],
)
def test_release_asset_mutations_are_rejected(old: str, new: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow().replace(old, new))


@pytest.mark.parametrize(
    "requirement",
    [
        "monthly-close-control-plane",
        "monthly-close-control-plane==0.1.1",
        "monthly-close-control-plane>=0.1.1",
        "monthly-close-control-plane==0.1.0",
    ],
)
def test_registry_style_install_requirements_are_rejected(requirement: str) -> None:
    text = _secure_workflow().replace(
        _install_command("0.1.1"), f"python -m pip install {requirement}"
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_workflow_package_version_must_match_project_version() -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow(), project_version="0.1.2")


@pytest.mark.parametrize("uses", ["./local-action", "${{ matrix.action }}"])
def test_local_actions_and_whole_value_expressions_are_allowed(uses: str) -> None:
    _assert_workflow(_secure_workflow() + f"      - uses: {uses}\n")


@pytest.mark.parametrize(
    "uses", ["actions/checkout@${{ matrix.ref }}", "example/unknown@" + "1" * 40]
)
def test_embedded_expressions_and_unreviewed_external_actions_are_rejected(uses: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow() + f"      - uses: {uses}\n")


def test_commented_unsafe_lines_are_not_active_configuration() -> None:
    text = _secure_workflow() + (
        "      # - uses: actions/checkout@main\n"
        "      # - run: python -m pip install monthly-close-control-plane\n"
    )

    _assert_workflow(text)


@pytest.mark.parametrize(
    ("old", "new"),
    [
        ("contents: read", "contents: write"),
        ("contents: read", "contents: read\n  actions: write"),
        ("persist-credentials: false", "persist-credentials: true"),
        ("Never commit a\n# client trial balance.", "Client data may be committed."),
    ],
)
def test_safety_control_mutations_are_rejected(old: str, new: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow().replace(old, new))
