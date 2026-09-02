from __future__ import annotations

from pathlib import Path, PurePosixPath
import re

import yaml
from yaml.nodes import MappingNode, ScalarNode, SequenceNode


ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = ROOT.parents[1]

EXPECTED_POLICY = """\
# Agent instructions

This repository is a local, deterministic review-pack generator. Preserve these
accounting and human-review boundaries:

- Preserve exactly the `PASS`, `REVIEW`, and `BLOCKED` pack states. For `review` and
  `workbench`, exit `0` only for `PASS`, exit `2` for `REVIEW` or `BLOCKED`, and exit
  `1` for malformed input, invalid command configuration or an unwritable output.
  The read-only `view` command exits `0` after verified display and `1` on verification
  failure.
- An acknowledgement records a human action only. It never changes a control status,
  approves or signs off a close, or proves that a period was closed.
- Keep client source files, workpapers, review notes and generated packs in a separate,
  access-controlled directory outside the checkout. Repository fixtures must remain
  fabricated.
- Parse and calculate money, balances, thresholds and tolerances with exact `Decimal`
  arithmetic, never binary floating point. Preserve fail-closed schema and integrity gates.
- Do not add network or live Xero access, credential or token handling, journal, payment
  or report mutation, approval or sign-off authority, period locking, or tax lodgement.
- Route release work through [RELEASING.md](RELEASING.md) and the existing GitHub Actions
  workflows. Never build or upload release assets by hand, and do not tag or publish
  without explicit action-time approval.
"""


def _normalise(document: str) -> str:
    return re.sub(r"\s+", " ", document).strip()


def _section(document: str, heading: str) -> str:
    match = re.search(
        rf"^## {re.escape(heading)}\s*$\n(.*?)(?=^## |\Z)",
        document,
        flags=re.MULTILINE | re.DOTALL,
    )
    assert match is not None, f"missing ## {heading} section"
    return match.group(1)


def _fenced_commands(section: str) -> list[str]:
    blocks = re.findall(r"```(?:bash|powershell)\n(.*?)```", section, flags=re.DOTALL)
    return [line.strip() for block in blocks for line in block.splitlines() if line.strip()]


def _without_fenced_commands(section: str) -> str:
    return re.sub(r"```(?:bash|powershell)\n.*?```", "", section, flags=re.DOTALL)


def _ci_text() -> str:
    return (REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")


def _workflow_run_gates(workflow: str) -> list[tuple[bool, str]]:
    gates: list[tuple[bool, str]] = []

    def visit(node: yaml.Node) -> None:
        if isinstance(node, MappingNode):
            for key, value in node.value:
                if isinstance(key, ScalarNode) and key.value == "run":
                    assert isinstance(value, ScalarNode), "workflow run value must be scalar"
                    gates.append((value.style in ("|", ">"), value.value.rstrip("\n")))
                else:
                    visit(value)
        elif isinstance(node, SequenceNode):
            for value in node.value:
                visit(value)

    root = yaml.compose(workflow)
    assert root is not None, "workflow must not be empty"
    visit(root)
    return gates


def _scalar_ci_commands() -> list[str]:
    commands = [
        command
        for multiline, command in _workflow_run_gates(_ci_text())
        if not multiline
    ]
    return list(dict.fromkeys(commands))


def _multiline_ci_gates() -> list[str]:
    return [
        command
        for multiline, command in _workflow_run_gates(_ci_text())
        if multiline
    ]


def _multiline_ci_commands(step_name: str) -> list[str]:
    lines = _ci_text().splitlines()
    markers = [
        index for index, line in enumerate(lines) if line == f"      - name: {step_name}"
    ]
    assert len(markers) == 1, f"expected one {step_name!r} workflow step"
    start = markers[0]
    end = next(
        (index for index in range(start + 1, len(lines)) if lines[index].startswith("      - ")),
        len(lines),
    )
    step = lines[start:end]
    run_markers = [index for index, line in enumerate(step) if line == "        run: |"]
    assert len(run_markers) == 1, "expected one multiline run block"

    active: list[str] = []
    for line in step[run_markers[0] + 1 :]:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if len(line) - len(line.lstrip()) <= 8:
            break
        active.append(stripped)

    commands: list[str] = []
    continued = ""
    for line in active:
        if line.endswith("\\"):
            continued += line[:-1].rstrip() + " "
        else:
            commands.append((continued + line).strip())
            continued = ""
    assert not continued, "workflow smoke command has an unterminated continuation"
    return commands


def _ci_smoke_contract() -> tuple[str, ...]:
    commands = _multiline_ci_commands("Install and smoke-test the built wheel outside the checkout")
    assert len(commands) == 5, "package smoke step must contain five active commands"

    venv = re.fullmatch(r"python -m venv (?P<path>/\S+)", commands[0])
    assert venv is not None, f"unexpected CI venv command: {commands[0]!r}"
    venv_path = PurePosixPath(venv["path"])
    assert venv_path == PurePosixPath("/tmp/venv")
    assert commands[1] == (
        f"{venv_path}/bin/pip install --no-index --find-links dist "
        "monthly-close-control-plane"
    )
    assert commands[2] == "cd /tmp"
    assert commands[3] == (
        f"{venv_path}/bin/close-control review "
        '--current "$GITHUB_WORKSPACE/packages/monthly-close-control-plane/examples/current_trial_balance.csv" '
        '--prior "$GITHUB_WORKSPACE/packages/monthly-close-control-plane/examples/prior_trial_balance.csv" '
        "--output pack || [ $? -eq 2 ]"
    )
    assert commands[4] == "test -f pack/close-review-pack.json"
    return (
        "venv:system-temp",
        "install:monthly-close-control-plane",
        "cwd:system-temp",
        "review:fabricated-current+prior:output=pack:accept=2",
        "exists:pack/close-review-pack.json",
    )


def _guidance_smoke_contract(commands: list[str]) -> tuple[str, ...]:
    script = "\n".join(commands)
    assert commands[0] == '$ErrorActionPreference = "Stop"'
    build = next(command for command in _scalar_ci_commands() if command.endswith("python -m build"))
    assert f'{build} --outdir "$artifactDir"' in commands
    assert '$wheels = @(Get-ChildItem -LiteralPath $artifactDir -Filter "*.whl" -File)' in commands
    assert '$wheel = $wheels[0].FullName' in commands
    assert '& "$smokeDir\\venv\\Scripts\\python.exe" -m pip install --no-index "$wheel"' in commands
    assert "Resolve-Path dist" not in script and "--find-links" not in script
    assert (
        '& "$smokeDir\\venv\\Scripts\\close-control.exe" review '
        '--current "$repoRoot\\examples\\current_trial_balance.csv" '
        '--prior "$repoRoot\\examples\\prior_trial_balance.csv" --output pack'
    ) in commands
    assert (
        'if ($LASTEXITCODE -ne 2) { throw "expected REVIEW exit 2, got $LASTEXITCODE" }'
    ) in commands
    assert (
        'if (-not (Test-Path "pack\\close-review-pack.json")) { throw "smoke pack missing" }'
    ) in commands
    assert script.count("try {") == 2 and script.count("finally {") == 2
    assert "Pop-Location" in commands
    assert 'Remove-Item -LiteralPath $smokeDir -Recurse -Force -ErrorAction SilentlyContinue' in commands
    assert 'Remove-Item -LiteralPath $artifactDir -Recurse -Force -ErrorAction SilentlyContinue' in commands
    return (
        "venv:system-temp",
        "install:monthly-close-control-plane",
        "cwd:system-temp",
        "review:fabricated-current+prior:output=pack:accept=2",
        "exists:pack/close-review-pack.json",
    )


def test_agents_preserves_exact_control_and_human_review_boundaries() -> None:
    guidance = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
    policy, separator, _remainder = guidance.partition("## Repository map")

    assert separator
    assert _normalise(policy) == _normalise(EXPECTED_POLICY)


def test_agents_links_existing_docs_and_tracks_scalar_ci_commands() -> None:
    guidance = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
    repository_map = _section(guidance, "Repository map")
    ci = _section(guidance, "CI gates")

    assert _normalise(repository_map) == _normalise(
        """\
        - [README.md](README.md) owns the review-pack, status and exit-code contracts.
        - [CONTRIBUTING.md](CONTRIBUTING.md) owns fixture, data-handling and pull-request rules.
        - [RELEASING.md](RELEASING.md) owns release preflight, tagging and verification.
        """
    )
    assert _fenced_commands(ci) == _scalar_ci_commands()
    assert _normalise(_without_fenced_commands(ci)) == _normalise(
        """\
        The fenced list records the unique single-line commands in
        `.github/workflows/ci.yml`. The multiline package-smoke gate is explained and
        matched semantically below without duplicating its shell body:
        """
    )
    for path in ("README.md", "CONTRIBUTING.md", "RELEASING.md"):
        assert (ROOT / path).is_file()


def test_workflow_parser_detects_new_non_python_and_multiline_run_gates() -> None:
    workflow = """\
steps:
  - run: pwsh -File scripts/check-policy.ps1
  - run: |2-
      npm ci
      npm test
  - run: >- # folded command
      cargo fmt --check &&
      cargo test
"""

    assert _workflow_run_gates(workflow) == [
        (False, "pwsh -File scripts/check-policy.ps1"),
        (True, "npm ci\nnpm test"),
        (True, "cargo fmt --check && cargo test"),
    ]


def test_windows_smoke_uses_one_fresh_wheel_and_always_cleans_up() -> None:
    guidance = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
    commands = _fenced_commands(_section(guidance, "Package smoke outside the checkout"))
    script = "\n".join(commands)

    assert 'python -m build --outdir "$artifactDir"' in script
    assert '$wheels = @(Get-ChildItem -LiteralPath $artifactDir -Filter "*.whl" -File)' in script
    assert 'if ($wheels.Count -ne 1) { throw "expected exactly one built wheel" }' in script
    assert '$wheel = $wheels[0].FullName' in script
    assert '-m pip install --no-index "$wheel"' in script
    assert "Resolve-Path dist" not in script
    assert "--find-links" not in script
    assert script.count("try {") == 2
    assert script.count("finally {") == 2
    assert "Pop-Location" in script
    assert 'Remove-Item -LiteralPath $smokeDir -Recurse -Force -ErrorAction SilentlyContinue' in script
    assert 'Remove-Item -LiteralPath $artifactDir -Recurse -Force -ErrorAction SilentlyContinue' in script


def test_agents_keeps_installed_wheel_smoke_outside_checkout() -> None:
    guidance = (ROOT / "AGENTS.md").read_text(encoding="utf-8")
    smoke = _section(guidance, "Package smoke outside the checkout")

    assert len(_multiline_ci_gates()) == 1
    assert _guidance_smoke_contract(_fenced_commands(smoke)) == _ci_smoke_contract()
    assert _normalise(_without_fenced_commands(smoke)) == _normalise(
        """\
        The CI smoke uses `/tmp`; on Windows use separate fresh system temporary artifact
        and smoke directories. Fail immediately if a native build or install command fails,
        install only the one wheel produced by that build, and always restore the caller's
        location and remove both temporary directories. The fabricated demo deliberately
        returns `REVIEW` exit `2`, which is the accepted smoke result. This proves the
        installed wheel, not a checkout import, and does not publish anything:
        """
    )


def test_claude_imports_shared_guidance_exactly() -> None:
    assert (ROOT / "CLAUDE.md").read_text(encoding="utf-8") == "@AGENTS.md\n"
