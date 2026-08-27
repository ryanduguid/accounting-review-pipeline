from __future__ import annotations

import re
import shlex
from collections import Counter
from pathlib import Path
from typing import Any

import pytest
import yaml
from yaml.constructor import ConstructorError
from yaml.nodes import MappingNode, ScalarNode, SequenceNode
from yaml.resolver import BaseResolver
from yaml.tokens import AliasToken, AnchorToken

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - exercised by the Python 3.10 CI job
    import tomli as tomllib


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"
WORKFLOW = EXAMPLES / "github-actions-close-check.yml"

REPOSITORY_NAME = "monthly-close-controls"
PACKAGE_NAME = "monthly-close-control-plane"
REPOSITORY_URL = f"https://github.com/ryanduguid/{REPOSITORY_NAME}"
RELEASE_ASSET_SHA256 = "e4ca2bce708a3e28c8a6316eae68095848a116a04a99f24bc1d7325d92a449d9"

# The committed example must point at a release that actually exists, so this
# tracks the last *published* tag rather than pyproject.toml's version (which
# is bumped ahead of the tag during release prep). Bump this, RELEASE_ASSET_SHA256
# and examples/github-actions-close-check.yml together in one follow-up commit,
# once the new tag's wheel is built and its real sha256 is confirmed
# (`sha256sum` against the downloaded release asset, not a locally-built guess:
# the wheel embeds SOURCE_DATE_EPOCH from the tag commit, so a local build
# before the tag exists cannot reproduce the real bytes).
EXAMPLE_PINNED_VERSION = "0.1.1"

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
PACKAGE_MENTION = re.compile(
    r"monthly[-_.]close[-_.]control[-_.]plane",
    re.IGNORECASE,
)
PIP_EXECUTABLE = re.compile(r"pip(?:\d+(?:\.\d+)*)?(?:\.exe)?", re.IGNORECASE)


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


def _requirement(project_version: str) -> str:
    return (
        f"{PACKAGE_NAME} @ "
        f"{REPOSITORY_URL}/releases/download/"
        f"v{project_version}/monthly_close_control_plane-{project_version}-py3-none-any.whl"
        f"#sha256={RELEASE_ASSET_SHA256}"
    )


def _install_command(project_version: str) -> str:
    return f'python -m pip install "{_requirement(project_version)}"'


def _load_workflow(text: str) -> dict[str, Any]:
    if any(
        isinstance(token, (AnchorToken, AliasToken))
        for token in yaml.scan(text, Loader=StrictWorkflowLoader)
    ):
        raise TypeError("workflow anchors and aliases are not allowed")
    loaded = yaml.load(text, Loader=StrictWorkflowLoader)
    if not isinstance(loaded, dict):
        raise TypeError("workflow root must be a mapping")
    return loaded


def _uses_scalar_nodes(text: str) -> list[ScalarNode]:
    root = yaml.compose(text, Loader=StrictWorkflowLoader)
    if root is None:
        raise TypeError("workflow must not be empty")

    found: list[ScalarNode] = []
    pending = [root]
    while pending:
        node = pending.pop()
        if isinstance(node, MappingNode):
            for key_node, value_node in node.value:
                if (
                    isinstance(key_node, ScalarNode)
                    and key_node.value == "uses"
                    and isinstance(value_node, ScalarNode)
                ):
                    found.append(value_node)
                pending.append(value_node)
        elif isinstance(node, SequenceNode):
            pending.extend(node.value)
    return found


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
    uses_nodes: list[ScalarNode],
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

    source_lines = text.splitlines()
    for action, (sha, release) in EXPECTED_EXTERNAL_PINS.items():
        reference = f"{action}@{sha}"
        matching_nodes = [node for node in uses_nodes if node.value == reference]
        labelled = False
        if len(matching_nodes) == 1:
            node = matching_nodes[0]
            if node.start_mark.line == node.end_mark.line:
                trailing_text = source_lines[node.end_mark.line][node.end_mark.column :]
                labelled = re.fullmatch(
                    rf"\s+#\s*{re.escape(release)}\s*",
                    trailing_text,
                ) is not None
        if not labelled:
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


def _pip_token_indices(tokens: list[str]) -> list[int]:
    return [
        index
        for index, token in enumerate(tokens)
        if PIP_EXECUTABLE.fullmatch(_executable_name(token))
    ]


def _validate_package_installs(
    run_commands: list[str],
    project_version: str,
    errors: list[str],
) -> None:
    pip_invocations: list[tuple[list[str], int]] = []
    ambiguous_mentions: list[str] = []
    for command in run_commands:
        try:
            tokens = _shell_tokens(command)
        except ValueError as exc:
            if PACKAGE_MENTION.search(command):
                errors.append(f"ambiguous package-related run command: {exc}")
            continue

        invocation_indices = _pip_token_indices(tokens)
        pip_invocations.extend((tokens, index) for index in invocation_indices)
        if not invocation_indices and PACKAGE_MENTION.search(" ".join(tokens)):
            ambiguous_mentions.append(command)

    expected_tokens = ["python", "-m", "pip", "install", _requirement(project_version)]
    if len(pip_invocations) != 1 or pip_invocations[0][0] != expected_tokens:
        errors.append(
            "the workflow must contain exactly one pip invocation and its complete "
            f"token vector must be {expected_tokens!r}; got {pip_invocations!r}"
        )
    if ambiguous_mentions:
        errors.append(
            "package or wheel references outside a recognised pip invocation are ambiguous: "
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
        uses_nodes = _uses_scalar_nodes(text)
    except (yaml.YAMLError, TypeError) as exc:
        return [f"workflow must be strict, duplicate-free YAML: {exc}"]

    expected_permissions = {"contents": "read"}
    if workflow.get("permissions") != expected_permissions:
        errors.append("top-level permissions must be exactly contents: read")

    uses_values, run_commands, steps = _workflow_surfaces(workflow, errors)
    _validate_uses(text, uses_values, uses_nodes, steps, errors)
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

    assert project_version == lock_version == release_version == "0.1.2"
    assert "PyYAML>=6.0.3,<7" in project["project"]["optional-dependencies"]["dev"]
    assert yaml_version == yaml.__version__ == "6.0.3"


def test_repository_identity_is_distinct_from_package_identity() -> None:
    project = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))

    assert project["project"]["name"] == PACKAGE_NAME
    assert project["project"]["urls"]["Repository"] == f"{REPOSITORY_URL}.git"


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
    _assert_workflow(
        WORKFLOW.read_text(encoding="utf-8"), project_version=EXAMPLE_PINNED_VERSION
    )


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
    "requirement",
    [
        _requirement("0.1.1").replace(PACKAGE_NAME, f"{PACKAGE_NAME}[dev]", 1),
        _requirement("0.1.1").partition(" @ ")[2],
    ],
)
def test_final_review_additional_pip_install_forms_are_rejected(
    requirement: str,
) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(
            _secure_workflow()
            + f'      - run: python -m pip install "{requirement}"\n'
        )


@pytest.mark.parametrize(
    "text",
    [
        _secure_workflow().replace("pip install", "pip install --no-deps"),
        _secure_workflow() + "      - run: pip3 install requests\n",
    ],
)
def test_pip_inventory_requires_one_exact_token_vector(text: str) -> None:
    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_final_review_release_label_is_bound_to_actual_uses_node() -> None:
    checkout, (sha, release) = next(iter(EXPECTED_EXTERNAL_PINS.items()))
    text = _secure_workflow().replace(
        f"{checkout}@{sha} # {release}",
        f"{checkout}@{sha}",
    ) + f"""      - name: Harmless provenance text
        run: |
          cat <<'EOF'
          uses: {checkout}@{sha} # {release}
          EOF
"""

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_alias_review_anchor_cannot_lend_its_label_to_a_uses_alias() -> None:
    checkout, (sha, release) = next(iter(EXPECTED_EXTERNAL_PINS.items()))
    text = _secure_workflow().replace(
        "on: workflow_dispatch",
        f"env:\n  CHECKOUT_REF: &checkout_ref {checkout}@{sha} # {release}\n"
        "on: workflow_dispatch",
    ).replace(
        f"uses: {checkout}@{sha} # {release}",
        "uses: *checkout_ref",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


def test_inverse_workflow_alias_label_case_is_rejected() -> None:
    checkout, (sha, release) = next(iter(EXPECTED_EXTERNAL_PINS.items()))
    text = _secure_workflow().replace(
        "on: workflow_dispatch",
        f"env:\n  CHECKOUT_REF: &checkout_ref {checkout}@{sha}\n"
        "on: workflow_dispatch",
    ).replace(
        f"uses: {checkout}@{sha} # {release}",
        f"uses: *checkout_ref # {release}",
    )

    with pytest.raises(AssertionError):
        _assert_workflow(text)


@pytest.mark.parametrize("reference_kind", ["anchor", "merge-alias"])
def test_yaml_reference_tokens_are_rejected(reference_kind: str) -> None:
    texts = {
        "anchor": _secure_workflow().replace(
            "on: workflow_dispatch",
            "name: &workflow_name Close check\non: workflow_dispatch",
        ),
        "merge-alias": _secure_workflow()
        .replace(
            "permissions:\n  contents: read",
            "permissions: &read_permissions\n  contents: read",
        )
        .replace(
            "  close-check:\n    steps:",
            "  close-check:\n    permissions:\n"
            "      <<: *read_permissions\n    steps:",
        ),
    }

    with pytest.raises(AssertionError):
        _assert_workflow(texts[reference_kind])


@pytest.mark.parametrize(
    "suffix",
    [
        "      # Literal &anchor and *alias text\n",
        "      - run: |\n          echo 'Literal &anchor and *alias text'\n",
        '      - name: "Literal &anchor and *alias text"\n        run: echo safe\n',
        "      - name: Literal A&B and 2*3\n        run: echo safe\n",
    ],
)
def test_yaml_reference_characters_in_scalar_text_are_allowed(suffix: str) -> None:
    _assert_workflow(_secure_workflow() + suffix)


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
        (
            REPOSITORY_URL.removeprefix("https://"),
            f"github.com/example/{REPOSITORY_NAME}",
        ),
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
