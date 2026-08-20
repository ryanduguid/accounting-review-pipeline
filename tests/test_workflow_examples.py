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

USES_KEY_LINE = re.compile(r"^\s*(?:-\s*)?uses:\s*(?P<value>.*?)\s*$")
RUN_KEY_LINE = re.compile(r"^\s*(?:-\s*)?run:\s*(?P<command>.+?)\s*$")
RUN_BLOCK_HEADER = re.compile(r"^(?P<indent> *)(?:-\s*)?run:\s*[>|][0-9+-]*\s*$")
BLOCK_SCALAR_HEADER = re.compile(
    r"^(?P<indent> *)(?:-\s*)?[A-Za-z0-9_.-]+:\s*[>|][0-9+-]*\s*$"
)
FLOW_MAPPING_LINE = re.compile(r"^\s*-\s*\{(?P<body>.*)\}\s*$")
PERSIST_CREDENTIALS_LINE = re.compile(
    r"^\s*persist-credentials:\s*(?P<value>.*?)\s*$"
)
PERMISSIONS_LINE = re.compile(
    r"^(?P<indent> *)(?P<sequence>-\s*)?permissions:\s*(?P<value>.*?)\s*$"
)
MAPPING_ENTRY = re.compile(
    r"^(?P<indent> *)(?P<key>[A-Za-z0-9_.-]+):\s*(?P<value>.*?)\s*$"
)
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


def _split_yaml_comment(line: str) -> tuple[str, str | None]:
    if line.startswith("#"):
        return "", line[1:].strip()
    marker = re.search(r"\s+#", line)
    if marker is None:
        return line.rstrip(), None
    return line[: marker.start()].rstrip(), line[marker.end() :].strip()


def _block_scalar_spans(lines: list[str]) -> dict[int, list[int]]:
    # This is deliberately a bounded structural scanner, not a YAML parser. It
    # recognises only the security-bearing shapes used by this copyable example
    # and fails closed when those keys use an unsupported shape.
    spans: dict[int, list[int]] = {}
    index = 0
    while index < len(lines):
        code, _ = _split_yaml_comment(lines[index])
        match = BLOCK_SCALAR_HEADER.fullmatch(code)
        if not match:
            index += 1
            continue

        header_indent = len(match.group("indent"))
        content: list[int] = []
        following = index + 1
        while following < len(lines):
            if not lines[following].strip():
                content.append(following)
                following += 1
                continue
            following_indent = len(lines[following]) - len(lines[following].lstrip())
            if following_indent <= header_indent:
                break
            content.append(following)
            following += 1
        spans[index] = content
        index = following
    return spans


def _scalar_content_lines(lines: list[str]) -> set[int]:
    return {
        line_number
        for content in _block_scalar_spans(lines).values()
        for line_number in content
    }


def _install_command(project_version: str) -> str:
    requirement = (
        f"{PACKAGE_NAME} @ "
        "https://github.com/ryanduguid/monthly-close-control-plane/releases/download/"
        f"v{project_version}/monthly_close_control_plane-{project_version}-py3-none-any.whl"
        f"#sha256={RELEASE_ASSET_SHA256}"
    )
    return f'python -m pip install "{requirement}"'


def _active_uses(text: str) -> list[tuple[int, str, str | None]]:
    lines = text.splitlines()
    scalar_content = _scalar_content_lines(lines)
    found: list[tuple[int, str, str | None]] = []
    for index, line in enumerate(lines):
        if index in scalar_content:
            continue
        code, comment = _split_yaml_comment(line)
        match = USES_KEY_LINE.fullmatch(code)
        if match:
            found.append(
                (
                    index + 1,
                    _unquote(match.group("value")),
                    comment,
                )
            )
            continue

        flow = FLOW_MAPPING_LINE.fullmatch(code)
        if not flow:
            continue
        body = flow.group("body")
        flow_uses = [
            _unquote(match.group("value"))
            for match in re.finditer(
                r"(?:^|,)\s*uses\s*:\s*(?P<value>.*?)(?=\s*,|$)", body
            )
        ]
        if re.search(r"(?:^|,)\s*uses\s*:", body) and not flow_uses:
            flow_uses = [code.strip()]
        found.extend((index + 1, value, comment) for value in flow_uses)
    return found


def _checkout_does_not_persist_credentials(text: str) -> bool:
    lines = text.splitlines()
    scalar_content = _scalar_content_lines(lines)
    checkout_lines: list[int] = []
    persist_lines: list[int] = []
    for index, line in enumerate(lines):
        if index in scalar_content:
            continue
        code, _ = _split_yaml_comment(line)
        uses = USES_KEY_LINE.fullmatch(code)
        if uses and _unquote(uses.group("value")).startswith("actions/checkout@"):
            checkout_lines.append(index)
        if PERSIST_CREDENTIALS_LINE.fullmatch(code):
            persist_lines.append(index)

    if len(checkout_lines) != 1:
        return False
    checkout_index = checkout_lines[0]
    checkout_code, _ = _split_yaml_comment(lines[checkout_index])
    if not checkout_code.lstrip().startswith("- uses:"):
        return False

    step_indent = len(checkout_code) - len(checkout_code.lstrip())
    step_lines: list[int] = []
    for index in range(checkout_index + 1, len(lines)):
        if index in scalar_content:
            continue
        code, _ = _split_yaml_comment(lines[index])
        if not code.strip():
            continue
        indent = len(code) - len(code.lstrip())
        if indent < step_indent or (indent == step_indent and code.lstrip().startswith("-")):
            break
        step_lines.append(index)

    with_lines = [
        index
        for index in step_lines
        if len(lines[index]) - len(lines[index].lstrip()) == step_indent + 2
        and _split_yaml_comment(lines[index])[0].strip() == "with:"
    ]
    if len(with_lines) != 1:
        return False

    with_index = with_lines[0]
    with_indent = step_indent + 2
    bound_persist_lines: list[int] = []
    for index in range(with_index + 1, len(lines)):
        if index in scalar_content:
            continue
        code, _ = _split_yaml_comment(lines[index])
        if not code.strip():
            continue
        indent = len(code) - len(code.lstrip())
        if indent <= with_indent:
            break
        match = PERSIST_CREDENTIALS_LINE.fullmatch(code)
        if indent == with_indent + 2 and match and _unquote(match.group("value")) == "false":
            bound_persist_lines.append(index)

    return len(bound_persist_lines) == 1 and persist_lines == bound_persist_lines


def _mapping_entries_after(
    lines: list[str],
    index: int,
    key_indent: int,
    scalar_content: set[int],
) -> list[tuple[str, str]] | None:
    entries: list[tuple[str, str]] = []
    for following in range(index + 1, len(lines)):
        if following in scalar_content:
            return None
        code, _ = _split_yaml_comment(lines[following])
        if not code.strip():
            continue
        indent = len(code) - len(code.lstrip())
        if indent <= key_indent:
            break
        match = MAPPING_ENTRY.fullmatch(code)
        if indent != key_indent + 2 or not match or not match.group("value"):
            return None
        entries.append((match.group("key"), _unquote(match.group("value"))))
    return entries


def _inline_mapping_entries(value: str) -> list[tuple[str, str]] | None:
    if re.fullmatch(r"\{\s*contents\s*:\s*read\s*\}", value):
        return [("contents", "read")]
    if value == "{}":
        return []
    return None


def _job_for_line(lines: list[str], index: int, scalar_content: set[int]) -> str | None:
    for preceding in range(index - 1, -1, -1):
        if preceding in scalar_content:
            continue
        code, _ = _split_yaml_comment(lines[preceding])
        if not code.strip():
            continue
        indent = len(code) - len(code.lstrip())
        if indent == 2:
            match = re.fullmatch(r"  (?P<job>[A-Za-z0-9_.-]+):\s*", code)
            return match.group("job") if match else None
        if indent == 0:
            return None
    return None


def _permissions_are_read_only(text: str) -> bool:
    lines = text.splitlines()
    scalar_content = _scalar_content_lines(lines)
    occurrences: list[tuple[int, int, bool, list[tuple[str, str]] | None]] = []
    for index, line in enumerate(lines):
        if index in scalar_content:
            continue
        code, _ = _split_yaml_comment(line)
        match = PERMISSIONS_LINE.fullmatch(code)
        if match:
            sequence = match.group("sequence") is not None
            key_indent = len(match.group("indent")) + (2 if sequence else 0)
            value = _unquote(match.group("value"))
            mapping = (
                _mapping_entries_after(lines, index, key_indent, scalar_content)
                if not value
                else _inline_mapping_entries(value)
            )
            occurrences.append((index, key_indent, sequence, mapping))
        elif re.search(r"(?:^|[\s{,])permissions\s*:", code):
            return False

    expected = [("contents", "read")]
    top_level = [item for item in occurrences if item[1] == 0 and not item[2]]
    if len(top_level) != 1 or top_level[0][3] != expected:
        return False

    job_permission_counts: Counter[str] = Counter()
    for index, key_indent, sequence, mapping in occurrences:
        if key_indent == 0 and not sequence:
            continue
        if sequence or key_indent != 4 or mapping != expected:
            return False
        job = _job_for_line(lines, index, scalar_content)
        if job is None:
            return False
        job_permission_counts[job] += 1
    return all(count == 1 for count in job_permission_counts.values())


def _active_package_installs(text: str) -> tuple[list[str], bool]:
    lines = text.splitlines()
    spans = _block_scalar_spans(lines)
    scalar_content = {
        line_number for content in spans.values() for line_number in content
    }
    commands: list[str] = []
    package_install_in_block = False
    for index, line in enumerate(lines):
        if index in scalar_content:
            continue
        code, _ = _split_yaml_comment(line)
        if RUN_BLOCK_HEADER.fullmatch(code):
            active_lines = [
                lines[line_number].strip()
                for line_number in spans.get(index, [])
                if lines[line_number].strip()
                and not lines[line_number].lstrip().startswith("#")
            ]
            active_block = "\n".join(active_lines)
            if "pip install" in active_block and PACKAGE_NAME in active_block:
                package_install_in_block = True
            continue

        run = RUN_KEY_LINE.fullmatch(code)
        if run:
            command = _unquote(run.group("command"))
            if "pip install" in command and PACKAGE_NAME in command:
                commands.append(command)
            continue

        flow = FLOW_MAPPING_LINE.fullmatch(code)
        if (
            flow
            and re.search(r"(?:^|,)\s*run\s*:", flow.group("body"))
            and "pip install" in flow.group("body")
            and PACKAGE_NAME in flow.group("body")
        ):
            commands.append("unsupported flow-style package install")
    return commands, package_install_in_block


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

    install_commands, package_install_in_block = _active_package_installs(text)
    expected_install = _install_command(project_version)
    if install_commands != [expected_install]:
        errors.append(
            "the active package install must be the one reviewed immutable release-asset "
            f"reference; expected {expected_install!r}, got {install_commands!r}"
        )
    if package_install_in_block:
        errors.append("package installation must not be hidden in a block-scalar run command")

    if not _permissions_are_read_only(text):
        errors.append(
            "top-level and effective job permissions must remain exactly contents: read"
        )
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


def test_adversarial_checkout_credentials_moved_to_env_are_rejected() -> None:
    text = _secure_workflow().replace(
        "        with:\n          persist-credentials: false",
        "        env:\n          persist-credentials: false",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_adversarial_job_permission_escalation_is_rejected() -> None:
    text = _secure_workflow().replace(
        "  close-check:\n    steps:",
        "  close-check:\n    permissions:\n      contents: write\n    steps:",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_adversarial_flow_style_external_action_is_rejected() -> None:
    text = _secure_workflow() + "      - {uses: actions/cache@v4}\n"

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_adversarial_block_scalar_unversioned_install_is_rejected() -> None:
    text = _secure_workflow() + (
        "      - name: Unsafe second install\n"
        "        run: |\n"
        "          python -m pip install monthly-close-control-plane\n"
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


@pytest.mark.parametrize(
    ("old", "new"),
    [
        (
            "        with:\n          persist-credentials: false\n",
            "",
        ),
        (
            "          persist-credentials: false",
            "          persist-credentials: false\n          persist-credentials: true",
        ),
    ],
)
def test_checkout_credentials_must_be_unique_and_inside_with(old: str, new: str) -> None:
    text = _secure_workflow().replace(old, new)
    if not new:
        text += "persist-credentials: false\n"

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_job_may_repeat_the_exact_read_only_permissions() -> None:
    text = _secure_workflow().replace(
        "  close-check:\n    steps:",
        "  close-check:\n    permissions:\n      contents: read\n    steps:",
    )

    _assert_workflow(text)


@pytest.mark.parametrize(
    "permission_mapping",
    [
        "permissions: write-all",
        "permissions: {}",
        "permissions:\n      contents: read\n      actions: write",
    ],
)
def test_job_permissions_must_not_override_the_exact_read_only_policy(
    permission_mapping: str,
) -> None:
    text = _secure_workflow().replace(
        "  close-check:\n    steps:",
        f"  close-check:\n    {permission_mapping}\n    steps:",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_step_permission_override_is_rejected() -> None:
    text = _secure_workflow().replace(
        "      - uses: actions/setup-python@",
        "      - permissions:\n          contents: write\n        uses: actions/setup-python@",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


@pytest.mark.parametrize("uses", ["./local-action", "'${{ matrix.action }}'"])
def test_flow_style_local_actions_and_whole_expressions_keep_their_exemption(
    uses: str,
) -> None:
    _assert_workflow(_secure_workflow() + f"      - {{uses: {uses}}}\n")


def test_a_second_reviewed_install_is_still_rejected() -> None:
    text = _secure_workflow() + (
        "      - name: Duplicate install\n"
        f"        run: {_install_command('0.1.1')}\n"
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_a_reviewed_install_in_a_block_scalar_is_rejected() -> None:
    text = _secure_workflow() + (
        "      - name: Duplicate block install\n"
        "        run: |\n"
        f"          {_install_command('0.1.1')}\n"
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_a_commented_install_inside_a_block_scalar_is_not_active() -> None:
    text = _secure_workflow() + (
        "      - name: Harmless block comment\n"
        "        run: |\n"
        "          # python -m pip install monthly-close-control-plane\n"
        "          echo done\n"
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
