from __future__ import annotations

import re
import shlex
from collections import Counter
from pathlib import Path
from typing import Any

import pytest
import yaml
from yaml.constructor import ConstructorError
from yaml.nodes import MappingNode
from yaml.resolver import BaseResolver

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - exercised by the Python 3.10 CI job
    import tomli as tomllib


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
WORKFLOW = EXAMPLES / "github-actions-close-check.yml"

PACKAGE_NAME = "monthly-close-control-plane"
NORMALISED_PACKAGE_NAME = "monthly-close-control-plane"
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

PINNED_ACTION = re.compile(
    r"(?P<action>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*)"
    r"@(?P<sha>[0-9a-fA-F]{40})"
)
WHOLE_EXPRESSION = re.compile(r"\$\{\{.+\}\}")
REQUIREMENT_NAME = re.compile(
    r"^(?P<name>[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?)"
    r"(?=\s*@|[<>=!~]|$)"
)
PIP_EXECUTABLE = re.compile(r"pip(?:\d+(?:\.\d+)*)?(?:\.exe)?", re.IGNORECASE)
PYTHON_EXECUTABLE = re.compile(
    r"python(?:\d+(?:\.\d+)*)?(?:\.exe)?", re.IGNORECASE
)
SHELL_OPERATOR = re.compile(r"[;&|]+")


class StrictWorkflowLoader(yaml.SafeLoader):
    """Safe loader with duplicate-key rejection and workflow-safe booleans."""


# PyYAML follows YAML 1.1 and otherwise treats the workflow key `on` as true.
# Keep booleans for actual true/false values while leaving on/off/yes/no as text.
StrictWorkflowLoader.yaml_implicit_resolvers = {
    first: [
        resolver
        for resolver in resolvers
        if resolver[0] != "tag:yaml.org,2002:bool"
    ]
    for first, resolvers in yaml.SafeLoader.yaml_implicit_resolvers.items()
}
StrictWorkflowLoader.add_implicit_resolver(
    "tag:yaml.org,2002:bool",
    re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"),
    list("tTfF"),
)


def _construct_unique_mapping(
    loader: StrictWorkflowLoader,
    node: MappingNode,
    deep: bool = False,
) -> dict[Any, Any]:
    loader.flatten_mapping(node)
    mapping: dict[Any, Any] = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        try:
            duplicate = key in mapping
        except TypeError as exc:
            raise ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                "found an unhashable mapping key",
                key_node.start_mark,
            ) from exc
        if duplicate:
            raise ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                f"found duplicate key {key!r}",
                key_node.start_mark,
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


StrictWorkflowLoader.add_constructor(
    BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_unique_mapping,
)


def _normalise_distribution_name(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def _requirement(project_version: str) -> str:
    return (
        f"{PACKAGE_NAME} @ "
        "https://github.com/ryanduguid/monthly-close-control-plane/releases/download/"
        f"v{project_version}/monthly_close_control_plane-{project_version}-py3-none-any.whl"
        f"#sha256={RELEASE_ASSET_SHA256}"
    )


def _install_command(project_version: str) -> str:
    return f'python -m pip install "{_requirement(project_version)}"'


def _load_workflow(text: str) -> dict[str, Any]:
    loaded = yaml.load(text, Loader=StrictWorkflowLoader)
    if not isinstance(loaded, dict):
        raise TypeError("workflow root must be a mapping")
    return loaded


def _workflow_surfaces(
    workflow: dict[str, Any],
    errors: list[str],
) -> tuple[list[tuple[str, Any]], list[str], list[dict[str, Any]]]:
    uses_values: list[tuple[str, Any]] = []
    run_commands: list[str] = []
    steps_seen: list[dict[str, Any]] = []
    jobs = workflow.get("jobs")
    if not isinstance(jobs, dict) or not jobs:
        errors.append("jobs must be a non-empty mapping")
        return uses_values, run_commands, steps_seen

    expected_permissions = {"contents": "read"}
    for job_name, job in jobs.items():
        if not isinstance(job_name, str) or not isinstance(job, dict):
            errors.append("every job must have a string name and mapping value")
            continue
        if "permissions" in job and job["permissions"] != expected_permissions:
            errors.append(
                f"job {job_name!r} permissions must be absent or exactly contents: read"
            )
        if "uses" in job:
            uses_values.append((f"job {job_name}", job["uses"]))

        steps = job.get("steps", [])
        if not isinstance(steps, list):
            errors.append(f"job {job_name!r} steps must be a list")
            continue
        for index, step in enumerate(steps):
            label = f"job {job_name} step {index + 1}"
            if not isinstance(step, dict):
                errors.append(f"{label} must be a mapping")
                continue
            steps_seen.append(step)
            if "permissions" in step:
                errors.append(f"{label} must not declare unsupported step permissions")
            if "uses" in step:
                uses_values.append((label, step["uses"]))
            if "run" in step:
                if isinstance(step["run"], str):
                    run_commands.append(step["run"])
                else:
                    errors.append(f"{label} run value must be a string")
    return uses_values, run_commands, steps_seen


def _validate_uses(
    text: str,
    uses_values: list[tuple[str, Any]],
    steps: list[dict[str, Any]],
    errors: list[str],
) -> None:
    actual_pins: list[tuple[str, str]] = []
    for label, value in uses_values:
        if not isinstance(value, str):
            errors.append(f"{label} uses value must be a string")
            continue
        if value.startswith("./") or WHOLE_EXPRESSION.fullmatch(value):
            continue
        match = PINNED_ACTION.fullmatch(value)
        if not match:
            errors.append(
                f"{label} external uses value must end in an exact 40-character commit SHA"
            )
            continue
        actual_pins.append((match.group("action"), match.group("sha")))

    expected_pins = [
        (action, sha) for action, (sha, _release) in EXPECTED_EXTERNAL_PINS.items()
    ]
    if Counter(actual_pins) != Counter(expected_pins):
        errors.append(
            "external action pins must exactly match the reviewed action/SHA set; "
            f"expected {expected_pins!r}, got {actual_pins!r}"
        )

    for action, (sha, release) in EXPECTED_EXTERNAL_PINS.items():
        pattern = re.compile(
            rf"(?m)^\s*(?:-\s*)?(?:uses|['\"]uses['\"]):\s*"
            rf"['\"]?{re.escape(action)}@{sha}['\"]?\s+#\s*{re.escape(release)}\s*$"
        )
        if len(pattern.findall(text)) != 1:
            errors.append(
                f"{action} must retain exactly one adjacent reviewed release comment {release}"
            )

    checkout_steps = [
        step
        for step in steps
        if isinstance(step.get("uses"), str)
        and step["uses"].startswith("actions/checkout@")
    ]
    if len(checkout_steps) != 1:
        errors.append("workflow must contain exactly one checkout step")
        return
    inputs = checkout_steps[0].get("with")
    if not isinstance(inputs, dict) or inputs.get("persist-credentials") is not False:
        errors.append("checkout with.persist-credentials must be the Boolean false")


def _shell_tokens(command: str) -> list[str]:
    lexer = shlex.shlex(command, posix=True, punctuation_chars=";&|")
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return list(lexer)


def _executable_name(token: str) -> str:
    return token.replace("\\", "/").rsplit("/", 1)[-1]


def _pip_install_invocations(tokens: list[str]) -> list[tuple[int, int]]:
    invocations: list[tuple[int, int]] = []
    index = 0
    while index < len(tokens):
        executable = _executable_name(tokens[index])
        if (
            PYTHON_EXECUTABLE.fullmatch(executable)
            and index + 3 < len(tokens)
            and tokens[index + 1] == "-m"
            and PIP_EXECUTABLE.fullmatch(_executable_name(tokens[index + 2]))
            and tokens[index + 3].lower() == "install"
        ):
            invocations.append((index, index + 4))
            index += 4
            continue
        if (
            PIP_EXECUTABLE.fullmatch(executable)
            and index + 1 < len(tokens)
            and tokens[index + 1].lower() == "install"
        ):
            invocations.append((index, index + 2))
            index += 2
            continue
        index += 1
    return invocations


def _requirement_name(token: str) -> str | None:
    match = REQUIREMENT_NAME.match(token)
    if not match:
        return None
    return _normalise_distribution_name(match.group("name"))


def _validate_package_installs(
    run_commands: list[str],
    project_version: str,
    errors: list[str],
) -> None:
    target_requirements: list[str] = []
    ambiguous_mentions: list[str] = []
    for command in run_commands:
        try:
            tokens = _shell_tokens(command)
        except ValueError as exc:
            if NORMALISED_PACKAGE_NAME in _normalise_distribution_name(command):
                errors.append(f"ambiguous package-related run command: {exc}")
            continue

        target_positions = {
            index
            for index, token in enumerate(tokens)
            if _requirement_name(token) == NORMALISED_PACKAGE_NAME
        }
        associated_positions: set[int] = set()
        invocations = _pip_install_invocations(tokens)
        for invocation_index, (_command_start, arguments_start) in enumerate(invocations):
            arguments_end = len(tokens)
            if invocation_index + 1 < len(invocations):
                arguments_end = invocations[invocation_index + 1][0]
            for index in range(arguments_start, arguments_end):
                if SHELL_OPERATOR.fullmatch(tokens[index]):
                    arguments_end = index
                    break
            for index in range(arguments_start, arguments_end):
                if index in target_positions:
                    associated_positions.add(index)
                    target_requirements.append(tokens[index])

        ambiguous_mentions.extend(
            tokens[index] for index in sorted(target_positions - associated_positions)
        )

    expected = _requirement(project_version)
    if target_requirements != [expected]:
        errors.append(
            "the active package install must be exactly one reviewed immutable "
            f"release-asset reference; expected {expected!r}, got {target_requirements!r}"
        )
    if ambiguous_mentions:
        errors.append(
            "package references outside a recognised pip install are ambiguous: "
            f"{ambiguous_mentions!r}"
        )


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
    try:
        workflow = _load_workflow(text)
    except (yaml.YAMLError, TypeError) as exc:
        return [f"workflow must be strict, duplicate-free YAML: {exc}"]

    expected_permissions = {"contents": "read"}
    if workflow.get("permissions") != expected_permissions:
        errors.append("top-level permissions must be exactly contents: read")

    uses_values, run_commands, steps = _workflow_surfaces(workflow, errors)
    _validate_uses(text, uses_values, steps, errors)
    _validate_package_installs(run_commands, project_version, errors)
    if not _has_client_data_warning(text):
        errors.append("the fabricated-data and never-commit-client-data warning must remain")
    return errors


def _assert_workflow(text: str, *, project_version: str = "0.1.1") -> None:
    errors = _workflow_errors(text, project_version=project_version)
    assert not errors, "\n".join(errors)


def _secure_workflow() -> str:
    return f"""# Store only fabricated or synthetic data in the repository. Never commit a
# client trial balance.
on: workflow_dispatch
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


def _repository_versions() -> tuple[str, str, str, str]:
    project = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((ROOT / "uv.lock").read_text(encoding="utf-8"))
    root_packages = [
        package
        for package in lock["package"]
        if package["name"] == PACKAGE_NAME and package.get("source") == {"editable": "."}
    ]
    yaml_packages = [package for package in lock["package"] if package["name"] == "pyyaml"]
    assert len(root_packages) == len(yaml_packages) == 1

    release_heading = (ROOT / "RELEASE_NOTES.md").read_text(encoding="utf-8").splitlines()[0]
    release_match = re.fullmatch(r"# v(?P<version>\d+\.\d+\.\d+)", release_heading)
    assert release_match, "RELEASE_NOTES.md must start with a semantic version heading"
    return (
        project["project"]["version"],
        root_packages[0]["version"],
        release_match.group("version"),
        yaml_packages[0]["version"],
    )


def test_copyable_workflow_discovery_is_not_vacuous() -> None:
    workflows = sorted({*EXAMPLES.rglob("*.yml"), *EXAMPLES.rglob("*.yaml")})

    assert workflows == [WORKFLOW]


def test_repository_versions_and_yaml_dev_dependency_agree() -> None:
    project_version, lock_version, release_version, yaml_version = _repository_versions()
    project = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))

    assert project_version == lock_version == release_version == "0.1.1"
    assert "PyYAML>=6.0.3,<7" in project["project"]["optional-dependencies"]["dev"]
    assert yaml_version == yaml.__version__ == "6.0.3"


def test_strict_loader_keeps_on_as_text_and_rejects_duplicate_keys() -> None:
    loaded = _load_workflow(_secure_workflow())
    duplicate = _secure_workflow().replace(
        "  contents: read", "  contents: read\n  contents: write", 1
    )

    assert "on" in loaded
    assert True not in loaded
    with pytest.raises(ConstructorError, match="duplicate key 'contents'"):
        _load_workflow(duplicate)


def test_committed_copyable_workflow_uses_only_reviewed_immutable_dependencies() -> None:
    project_version, _, _, _ = _repository_versions()

    _assert_workflow(WORKFLOW.read_text(encoding="utf-8"), project_version=project_version)


@pytest.mark.parametrize("action", EXPECTED_EXTERNAL_PINS)
@pytest.mark.parametrize("replacement_kind", ["tag", "main", "short-sha", "other-sha"])
def test_action_pin_mutations_are_rejected(action: str, replacement_kind: str) -> None:
    sha, release = EXPECTED_EXTERNAL_PINS[action]
    replacements = {
        "tag": release,
        "main": "main",
        "short-sha": sha[:-1],
        "other-sha": "0" * 40,
    }

    with pytest.raises(AssertionError):
        _assert_workflow(
            _secure_workflow().replace(
                f"{action}@{sha}", f"{action}@{replacements[replacement_kind]}"
            )
        )


@pytest.mark.parametrize("action", EXPECTED_EXTERNAL_PINS)
@pytest.mark.parametrize("mutation", ["remove", "alter"])
def test_action_release_comment_mutations_are_rejected(action: str, mutation: str) -> None:
    sha, release = EXPECTED_EXTERNAL_PINS[action]
    original = f"{action}@{sha} # {release}"
    suffix = "" if mutation == "remove" else f" # {release}-changed"

    with pytest.raises(AssertionError):
        _assert_workflow(
            _secure_workflow().replace(original, f"{action}@{sha}{suffix}")
        )


@pytest.mark.parametrize(
    "step",
    [
        "      - uses: ./local-action\n",
        "      - uses: ${{ matrix.action }}\n",
        "      - {uses: ./local-action}\n",
        "      - {'uses': '${{ matrix.action }}'}\n",
    ],
)
def test_local_actions_and_whole_value_expressions_are_allowed(step: str) -> None:
    _assert_workflow(_secure_workflow() + step)


@pytest.mark.parametrize(
    "step",
    [
        "      - uses: actions/checkout@${{ matrix.ref }}\n",
        "      - uses: example/unknown@" + "1" * 40 + "\n",
        '      - "uses": actions/checkout@v4\n',
        '      - {"uses": actions/checkout@v4}\n',
    ],
)
def test_unreviewed_external_action_forms_are_rejected(step: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow() + step)


@pytest.mark.parametrize(
    ("old", "new", "suffix"),
    [
        (
            "        with:\n          persist-credentials: false",
            "        env:\n          persist-credentials: false",
            "",
        ),
        (
            "        with:\n          persist-credentials: false\n",
            "",
            "persist-credentials: false\n",
        ),
        (
            "          persist-credentials: false",
            "          persist-credentials: false\n          persist-credentials: true",
            "",
        ),
        ("persist-credentials: false", 'persist-credentials: "false"', ""),
    ],
)
def test_checkout_credentials_must_be_boolean_false_in_with(
    old: str, new: str, suffix: str
) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow().replace(old, new) + suffix)


@pytest.mark.parametrize(
    "permission_override",
    [
        "    permissions:\n      contents: write\n",
        "    permissions: write-all\n",
        "    permissions: {}\n",
        "    permissions:\n      contents: read\n      actions: write\n",
        '    "permissions": {contents: write}\n',
        '    "permissions":\n      contents: write\n',
    ],
)
def test_job_permission_overrides_must_not_broaden_policy(
    permission_override: str,
) -> None:
    text = _secure_workflow().replace(
        "  close-check:\n    steps:",
        f"  close-check:\n{permission_override}    steps:",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


@pytest.mark.parametrize(
    "permission_override",
    [
        "    permissions:\n      contents: read\n",
        '    "permissions": {contents: read}\n',
    ],
)
def test_job_may_repeat_exact_read_only_permissions(permission_override: str) -> None:
    text = _secure_workflow().replace(
        "  close-check:\n    steps:",
        f"  close-check:\n{permission_override}    steps:",
    )

    _assert_workflow(text)


def test_step_permissions_and_duplicate_step_keys_are_rejected() -> None:
    step_permissions = _secure_workflow().replace(
        "      - uses: actions/setup-python@",
        "      - permissions:\n          contents: write\n        uses: actions/setup-python@",
    )
    duplicate_uses = _secure_workflow().replace(
        "      - uses: actions/setup-python@",
        "      - uses: actions/checkout@v4\n        uses: actions/setup-python@",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(step_permissions)
    with pytest.raises(AssertionError, match="duplicate key 'uses'"):
        _assert_workflow(duplicate_uses)


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


@pytest.mark.parametrize(
    "step",
    [
        "      - run: python -m pip  install monthly-close-control-plane\n",
        "      - run: pip3 install monthly-close-control-plane\n",
        "      - run: python -m pip install monthly_close_control_plane\n",
        "      - run: python -m pip install Monthly-Close-Control-Plane\n",
        '      - "run": python -m pip install monthly-close-control-plane\n',
        '      - {"run": "python -m pip install monthly-close-control-plane"}\n',
        "      - run: |\n          python -m pip install monthly-close-control-plane\n",
        f"      - run: {_install_command('0.1.1')}\n",
    ],
)
def test_additional_equivalent_package_installs_are_rejected(step: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow() + step)


@pytest.mark.parametrize(
    "step",
    [
        "      # - run: python -m pip install monthly-close-control-plane\n",
        "      - run: |\n          # python -m pip install monthly-close-control-plane\n          echo done\n",
        "      - run: |\n          echo done # python -m pip install monthly-close-control-plane\n",
    ],
)
def test_commented_package_installs_are_inactive(step: str) -> None:
    _assert_workflow(_secure_workflow() + step)


def test_workflow_package_version_must_match_project_version() -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow(), project_version="0.1.2")


@pytest.mark.parametrize(
    ("old", "new"),
    [
        ("contents: read", "contents: write"),
        ("contents: read", "contents: read\n  actions: write"),
        ("Never commit a\n# client trial balance.", "Client data may be committed."),
    ],
)
def test_safety_control_mutations_are_rejected(old: str, new: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(_secure_workflow().replace(old, new))
