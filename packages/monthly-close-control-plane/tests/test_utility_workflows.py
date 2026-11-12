# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The portable runner refuses destructive destinations before running commands."""
import json
import runpy
import shutil
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).parents[1]
MODULE = runpy.run_path(str(ROOT / "examples/utility_workflows.py"))
PREPARE = MODULE["prepare"]


def test_missing_companion_entrypoint_fails_before_creating_outputs(tmp_path):
    accounting = tmp_path / "accounting"
    wip = accounting / "packages/the-wip-tally"
    fpa = tmp_path / "fpa"
    for project in (wip, fpa):
        project.mkdir(parents=True)
        (project / "pyproject.toml").write_text("[project]")
    output, environments = tmp_path / "output", tmp_path / "environments"
    with pytest.raises(ValueError, match="job_to_cash.py.*#236"):
        PREPARE(output, fpa, accounting, None, environments, "job-cash")
    assert not output.exists() and not environments.exists()


def test_existing_output_is_not_overwritten(tmp_path):
    (tmp_path / "pyproject.toml").write_text("[project]")
    output = tmp_path / "existing"
    output.mkdir()
    with pytest.raises(ValueError, match="outside|new output"):
        PREPARE(output, tmp_path, None, None, None, "quarter")


def test_missing_sibling_is_explicit(tmp_path):
    with pytest.raises(ValueError, match="checkouts"):
        PREPARE(tmp_path / "new", tmp_path, None, None, None, "all")


def test_source_tree_is_never_an_output(tmp_path):
    (tmp_path / "pyproject.toml").write_text("[project]")
    with pytest.raises(ValueError, match="outside"):
        PREPARE(tmp_path / "generated", tmp_path, None, None, None, "quarter")


@pytest.mark.parametrize("layout", ["same", "inside_output", "contains_output"])
def test_fresh_environments_cannot_overlap_outputs(tmp_path, layout):
    source = tmp_path / "source"
    source.mkdir()
    (source / "pyproject.toml").write_text("[project]")
    output = tmp_path / "results"
    environments = {"same": output, "inside_output": output / "envs",
                    "contains_output": tmp_path}[layout]
    with pytest.raises(ValueError, match="separate new directory"):
        PREPARE(output, source, None, None, environments, "quarter")


@pytest.mark.skipif(sys.version_info < (3, 14), reason="The cross-repository driver requires Python 3.14")
def test_provenance_uses_workspace_lock_and_tracks_uncommitted_source(tmp_path):
    MODULE["git_output"](tmp_path, "init")
    (tmp_path / "pyproject.toml").write_text('[tool.uv.workspace]\nmembers = ["packages/*"]\n')
    (tmp_path / "uv.lock").write_text("workspace resolution")
    child = tmp_path / "packages/component"
    child.mkdir(parents=True)
    (child / "pyproject.toml").write_text('[project]\nname = "component"\n')
    (child / "uv.lock").write_text("standalone resolution")
    source = child / "example.py"
    source.write_text("first = 1\n")
    before = MODULE["project_evidence"](child)
    assert before["lockfile"] == "uv.lock"
    assert before["lock_scope"] == "workspace"
    assert before["revision"] is None
    source.write_text("first = 2\n")
    after = MODULE["project_evidence"](child)
    assert before["source_sha256"]["packages/component/example.py"] != after["source_sha256"]["packages/component/example.py"]


def test_command_timeout_stops_a_process(tmp_path):
    with pytest.raises(subprocess.TimeoutExpired):
        MODULE["captured"]([sys.executable, "-c", "import time; time.sleep(30)"], cwd=tmp_path, timeout=0.1)


def test_native_project_provenance_uses_its_cargo_lock(tmp_path):
    MODULE["git_output"](tmp_path, "init")
    (tmp_path / "pyproject.toml").write_text('[tool.uv.workspace]\nmembers = ["packages/*"]\n')
    (tmp_path / "uv.lock").write_text("Python workspace resolution")
    child = tmp_path / "packages/native"
    child.mkdir(parents=True)
    (child / "Cargo.toml").write_text('[package]\nname = "fixture"\n')
    (child / "Cargo.lock").write_text("Native resolution")
    evidence = MODULE["project_evidence"](child)
    assert evidence["lockfile"] == "packages/native/Cargo.lock"
    assert evidence["lock_scope"] == "project"


def test_every_route_requires_cargo_before_creating_outputs(tmp_path, monkeypatch):
    source = tmp_path / "source"
    source.mkdir()
    (source / "pyproject.toml").write_text("[project]")
    monkeypatch.setattr(MODULE["shutil"], "which", lambda name: None if name == "cargo" else "fixture")
    output = tmp_path / "output"
    with pytest.raises(ValueError, match="cargo"):
        PREPARE(output, source, None, None, None, "quarter")
    assert not output.exists()


def test_native_runtime_probe_keeps_the_close_process_boundary(tmp_path, monkeypatch):
    run = MODULE["run_workflows"]
    scope = run.__globals__
    output = tmp_path / "output"
    monkeypatch.setitem(scope, "prepare", lambda *args: {"projects": {"close": ROOT, "fpa": tmp_path}, "output": output, "environment_root": None})
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "fixture"})
    monkeypatch.setattr(scope["shutil"], "which", lambda name: name)
    commands = []

    def completed(command, **kwargs):
        commands.append(command)
        if "close-control" in command:
            return subprocess.CompletedProcess(command, 0, "close-control 1.0.0\n", "")
        if command[-1] == "--version":
            return subprocess.CompletedProcess(command, 0, "uv fixture", "")
        if "-c" in command:
            return subprocess.CompletedProcess(command, 0, "{}", "")
        quarter = output / "quarter"
        quarter.mkdir()
        (quarter / "quarter.json").write_text('{"periods": []}')
        return subprocess.CompletedProcess(command, 0, "", "")

    monkeypatch.setitem(scope, "captured", completed)
    with pytest.raises(ValueError, match="Fixture result validation failed"):
        run(output=output, fpa=tmp_path, workflow="quarter")
    assert commands[0] == ["cargo", "run", "--locked", "--release", "--quiet", "--bin", "close-control", "--", "--version"]
    calls = json.loads((output / "failed-results.json").read_text())["calls"]
    assert sum(call["kind"] == "runtime" for call in calls) == 2


def test_git_evidence_runs_plain_git_even_with_rtk_on_path(tmp_path, monkeypatch):
    scope = MODULE["git_output"].__globals__
    commands = []
    monkeypatch.setattr(scope["shutil"], "which", lambda name: name)

    def completed(command, **kwargs):
        commands.append(command)
        return subprocess.CompletedProcess(command, 0, "fixture revision", "")

    monkeypatch.setitem(scope, "captured", completed)
    assert MODULE["git_output"](tmp_path, "rev-parse", "HEAD") == "fixture revision"
    assert commands == [["git", "rev-parse", "HEAD"]]


@pytest.mark.parametrize("failure", ["exit", "timeout", "launch"])
def test_failed_steps_preserve_diagnostics_and_stop(tmp_path, monkeypatch, failure):
    run = MODULE["run_workflows"]
    scope = run.__globals__
    output = tmp_path / "results"
    monkeypatch.setitem(scope, "prepare", lambda *args: {"projects": {"fpa": tmp_path}, "output": output, "environment_root": None})
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "fixture"})

    def failed(*args, **kwargs):
        if failure == "timeout":
            raise subprocess.TimeoutExpired("fixture", 1, output="partial output", stderr="timeout detail")
        if failure == "launch":
            raise OSError("fixture executable unavailable")
        return subprocess.CompletedProcess("fixture", 1, "partial output", "fixture failure")

    monkeypatch.setitem(scope, "captured", failed)
    with pytest.raises(ValueError, match="failed"):
        run(output=output, fpa=tmp_path, workflow="quarter")
    record = json.loads((output / "failed-calls.json").read_text())
    assert len(record["calls"]) == 1
    assert record["stderr"]
    assert record["projects"]["fpa"]["revision"] == "fixture"
    replay = json.loads((output / "replay.json").read_text())
    assert replay["projects"] == record["projects"] and replay["workflow"] == "quarter"
    assert not (output / "manifest.json").exists()


def test_empty_quarter_never_writes_a_success_manifest(tmp_path, monkeypatch):
    run = MODULE["run_workflows"]
    scope = run.__globals__
    output = tmp_path / "results"
    monkeypatch.setitem(scope, "prepare", lambda *args: {"projects": {"fpa": tmp_path}, "output": output, "environment_root": None})
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "fixture"})

    def completed(command, **kwargs):
        if command[-1] == "--version":
            text = "uv fixture"
        elif "-c" in command:
            text = json.dumps({"python": "fixture", "packages": []})
        else:
            quarter = output / "quarter"
            quarter.mkdir()
            (quarter / "quarter.json").write_text('{"periods": []}')
            text = "complete"
        return subprocess.CompletedProcess(command, 0, text, "")

    monkeypatch.setitem(scope, "captured", completed)
    with pytest.raises(ValueError, match="Fixture result validation failed"):
        run(output=output, fpa=tmp_path, workflow="quarter")
    assert not (output / "manifest.json").exists()
    assert "three monthly periods" in json.loads((output / "failed-results.json").read_text())["error"]
    assert "Run failed" in (output / "summary.md").read_text()


@pytest.mark.parametrize("failure", ["runtime", "handoff", "workpaper", "hash", "manifest", "interrupt", "replace"])
def test_output_and_finalisation_failures_retain_failure_summary(tmp_path, monkeypatch, failure):
    run = MODULE["run_workflows"]
    scope = run.__globals__
    output = tmp_path / "results"
    projects = {name: tmp_path for name in ("fpa", "close", "grants")}
    monkeypatch.setitem(scope, "prepare", lambda *args: {"projects": projects, "output": output, "environment_root": None})
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "fixture"})

    def completed(command, **kwargs):
        if command[-1] == "--version":
            return subprocess.CompletedProcess(command, 0, "close-control 1.0.0\n" if "close-control" in command else "uv fixture", "")
        if "-c" in command:
            return subprocess.CompletedProcess(command, 0, "invalid JSON" if failure == "runtime" else "{}", "")
        if "grant-workpaper" in command:
            if failure != "workpaper":
                target = output / "grants"
                target.mkdir()
                (target / "workpaper.json").write_text("{}")
            return subprocess.CompletedProcess(command, 2, "", "")
        return subprocess.CompletedProcess(command, 2 if "review" in command else 0, "{}", "")

    monkeypatch.setitem(scope, "captured", completed)
    if failure in {"hash", "manifest", "interrupt", "replace"}:
        # Isolate finalisation faults after validation, without replacing filesystem behaviour elsewhere.
        monkeypatch.setitem(scope["RESULTS"], "validate", lambda *args: ["Verified fixture"])
        original_read = Path.read_bytes
        original_write = Path.write_text
        original_replace = Path.replace

        def read(path):
            if failure == "hash" and path == output / "summary.md":
                raise OSError("fixture hash read failure")
            return original_read(path)

        def write(path, data, *args, **kwargs):
            if failure in {"manifest", "interrupt"} and path == output / "manifest.json.tmp":
                original_write(path, "partial manifest")
                if failure == "interrupt":
                    raise KeyboardInterrupt
                raise OSError("fixture manifest write failure")
            return original_write(path, data, *args, **kwargs)

        def replace(path, target):
            if failure == "replace" and path == output / "manifest.json.tmp":
                raise OSError("fixture manifest rename failure")
            return original_replace(path, target)

        monkeypatch.setattr(Path, "read_bytes", read)
        monkeypatch.setattr(Path, "write_text", write)
        monkeypatch.setattr(Path, "replace", replace)
    workflow = "close-forecast" if failure == "handoff" else "grant-cash"
    with pytest.raises(ValueError, match="Fixture result validation failed"):
        run(output=output, fpa=tmp_path, grants=tmp_path, workflow=workflow)
    assert not (output / "manifest.json").exists()
    assert not (output / "manifest.json.tmp").exists()
    assert (output / "failed-results.json").is_file()
    summary = (output / "summary.md").read_text()
    assert "Run failed" in summary and "Fixture checks passed" not in summary



def test_invalid_timeout_fails_before_creating_output(tmp_path):
    with pytest.raises(ValueError, match="timeout"):
        MODULE["run_workflows"](output=tmp_path / "results", fpa=tmp_path, timeout=0)
    assert not (tmp_path / "results").exists()


@pytest.mark.parametrize("destination", ["output", "environment"])
def test_linked_wip_sources_are_protected(tmp_path, destination):
    accounting, fpa, target = (tmp_path / name for name in ("accounting", "fpa", "external-wip"))
    (accounting / "packages").mkdir(parents=True)
    for project in (fpa, target):
        project.mkdir()
        (project / "pyproject.toml").write_text("[project]")
    try:
        (accounting / "packages/the-wip-tally").symlink_to(target, target_is_directory=True)
    except OSError as exc:
        pytest.skip(f"Directory links unavailable: {exc}")
    output = target / "output" if destination == "output" else tmp_path / "output"
    env = target / "env" if destination == "environment" else None
    with pytest.raises(ValueError, match="outside"):
        PREPARE(output, fpa, accounting, None, env, "job-cash")
    assert not output.exists()


def _wip_checkout(root, *, native):
    accounting = root / "accounting"
    wip = accounting / "packages/the-wip-tally"
    wip.mkdir(parents=True)
    if native:
        (wip / "Cargo.toml").write_text('[package]\nname = "the-wip-tally"\n')
        (wip / "examples").mkdir()
        (wip / "examples/job_to_cash.rs").write_text("fn main() {}\n")
    else:
        (wip / "pyproject.toml").write_text("[project]")
        (wip / "examples").mkdir()
        (wip / "examples/job_to_cash.py").write_text("print('fixture')\n")
    return accounting, wip


def test_native_wip_manifest_is_accepted(tmp_path):
    accounting, wip = _wip_checkout(tmp_path, native=True)
    fpa = tmp_path / "fpa"
    fpa.mkdir()
    (fpa / "pyproject.toml").write_text("[project]")
    output = tmp_path / "output"
    prepared = PREPARE(output, fpa, accounting, None, None, "job-cash")
    assert prepared["projects"]["wip"] == wip.resolve()
    assert not output.exists()


def test_native_wip_without_example_fails_before_outputs(tmp_path):
    accounting, wip = _wip_checkout(tmp_path, native=True)
    (wip / "examples/job_to_cash.rs").unlink()
    fpa = tmp_path / "fpa"
    fpa.mkdir()
    (fpa / "pyproject.toml").write_text("[project]")
    output = tmp_path / "output"
    with pytest.raises(ValueError, match="job_to_cash.rs"):
        PREPARE(output, fpa, accounting, None, None, "job-cash")
    assert not output.exists()


@pytest.mark.parametrize("native", [True, False])
def test_wip_runner_follows_the_package_manifest(tmp_path, monkeypatch, native):
    run = MODULE["run_workflows"]
    scope = run.__globals__
    output = tmp_path / "output"
    wip = tmp_path / "wip"
    wip.mkdir()
    if native:
        (wip / "Cargo.toml").write_text('[package]\nname = "the-wip-tally"\n')
    projects = {"close": ROOT, "fpa": tmp_path, "wip": wip}
    monkeypatch.setitem(scope, "prepare", lambda *args: {"projects": projects, "output": output, "environment_root": None})
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "fixture"})
    monkeypatch.setattr(scope["shutil"], "which", lambda name: name)
    commands = []

    def completed(command, **kwargs):
        commands.append(command)
        if "--output" in command:
            Path(command[command.index("--output") + 1]).mkdir(parents=True, exist_ok=True)
        if "wip-tally" in command:
            return subprocess.CompletedProcess(command, 0, "wip-tally 1.0.0\n", "")
        if "close-control" in command:
            return subprocess.CompletedProcess(command, 0, "close-control 1.0.0\n", "")
        if command[-1] == "--version":
            return subprocess.CompletedProcess(command, 0, "uv fixture", "")
        if "-c" in command:
            return subprocess.CompletedProcess(command, 0, "{}", "")
        return subprocess.CompletedProcess(command, 0, "", "")

    monkeypatch.setitem(scope, "captured", completed)
    with pytest.raises(ValueError, match="Fixture result validation failed"):
        run(output=output, fpa=tmp_path, accounting=tmp_path, workflow="job-cash")
    if native:
        assert commands[2] == ["cargo", "run", "--locked", "--quiet", "--bin", "wip-tally", "--", "--version"]
        assert [command for command in commands if "--example" in command] == [
            ["cargo", "run", "--locked", "--quiet", "--example", "job_to_cash", "--",
             "--output", str(output / "job-base"), "--extra-cost-to-complete", "0"],
            ["cargo", "run", "--locked", "--quiet", "--example", "job_to_cash", "--",
             "--output", str(output / "job-extra-cost"), "--extra-cost-to-complete", "100000"],
        ]
    else:
        script = [command for command in commands if "examples/job_to_cash.py" in command]
        assert script == [
            ["uv", "run", "--project", str(wip), "--locked", "--python", "3.14", "python",
             "examples/job_to_cash.py", "--output", str(output / "job-base"), "--extra-cost-to-complete", "0"],
            ["uv", "run", "--project", str(wip), "--locked", "--python", "3.14", "python",
             "examples/job_to_cash.py", "--output", str(output / "job-extra-cost"), "--extra-cost-to-complete", "100000"],
        ]
        assert all("wip-tally" not in part for command in commands for part in command)


@pytest.fixture
def workflow_producers(results, monkeypatch):
    """Fabricated command results, with real result validation and file finalisation."""
    run = MODULE["run_workflows"]
    scope = run.__globals__
    close = results / "pipeline/packages/close"
    fpa, accounting, grants = (results / name for name in ("fpa-source", "accounting-source", "grant-source"))
    accounting, wip = _wip_checkout(accounting, native=True)
    for project, manifest in ((close, "Cargo.toml"), (fpa, "pyproject.toml"), (grants, "pyproject.toml")):
        project.mkdir(parents=True)
        (project / manifest).write_text("[project]" if manifest == "pyproject.toml" else "[package]")
    output = results / "actual-results"
    environment = results / "fresh-environments"
    commands = []
    monkeypatch.setitem(scope, "COMPONENT", close)
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "a" * 40, "source": str(project)})
    monkeypatch.setattr(scope["shutil"], "which", lambda name: name)

    def completed(command, **kwargs):
        commands.append((command, kwargs))
        text, exit_code = "", 0
        if command[-1] == "--version":
            text = "close-control 1.0.0\n" if "close-control" in command else "wip-tally 1.0.0\n" if "wip-tally" in command else "uv fixture\n"
        elif "-c" in command:
            text = '{"python":"fixture","packages":[]}'
        elif "compare" in command:
            period = Path(command[command.index("--current-pack") + 1]).parent.name
            text = (results / "quarter" / period / "comparison.json").read_text()
        elif "review" in command:
            exit_code = 2
            target = Path(command[command.index("--output") + 1])
            if not target.exists():
                shutil.copytree(results / "lumbridge-close", target)
        elif "--output" in command:
            target = Path(command[command.index("--output") + 1])
            if "close_handoff.py" in " ".join(command):
                target.mkdir()
                (target / "handoff.json").write_text(json.dumps({"source_sha256": dict.fromkeys(
                    ("xero_bs.csv", "invoices.csv", "payments.csv"), "fabricated-source")}))
            else:
                label = target.name
                shutil.copytree(results / label, target)
                if label == "grants":
                    exit_code = 2
                if label == "lumbridge-refresh":
                    path = target / "review.json"
                    document = json.loads(path.read_text())
                    document["source_sha256"] = dict.fromkeys(("xero_bs.csv", "invoices.csv", "payments.csv"), "fabricated-source")
                    path.write_text(json.dumps(document))
        elif "project_cash.py" in " ".join(command):
            target = Path(command[command.index("--pack") + 1])
            text = (target / "project-cash.json").read_text()
        elif "grant_cash.py" in " ".join(command):
            text = (results / "grant-cash.json").read_text()
        return subprocess.CompletedProcess(command, exit_code, text, "")

    monkeypatch.setitem(scope, "captured", completed)
    return {"run": run, "scope": scope, "completed": completed, "commands": commands,
            "arguments": {"output": output, "fpa": fpa, "accounting": accounting, "grants": grants,
                          "environment_root": environment}, "wip": wip}


def test_all_fabricated_routes_validate_commands_provenance_and_output_hashes(workflow_producers):
    fixture = workflow_producers
    manifest = fixture["run"](**fixture["arguments"])
    output = fixture["arguments"]["output"]
    assert manifest["fixture_validation"] == "passed"
    assert manifest["workflow"] == "all"
    assert json.loads((output / "manifest.json").read_text()) == manifest
    assert manifest["outputs_sha256"]["summary.md"]
    assert "Fixture checks passed" in (output / "summary.md").read_text()
    comparisons = [command for command, _ in fixture["commands"] if "compare" in command]
    assert {(Path(command[command.index("--previous-pack") + 1]).parent.name,
             Path(command[command.index("--current-pack") + 1]).parent.name) for command in comparisons} == {
                 ("2026-10-31", "2026-11-30"), ("2026-11-30", "2026-12-31")}
    environments = {options["env"]["UV_PROJECT_ENVIRONMENT"] for _, options in fixture["commands"] if options.get("env")}
    assert environments == {str(fixture["arguments"]["environment_root"] / owner) for owner in ("close", "fpa", "wip", "grants")}
    assert not (output / "failed-results.json").exists()


@pytest.mark.parametrize("failure", ["close-version", "wip-version", "uv-version", "source-handoff", "provenance"])
def test_failed_runtime_or_source_identity_never_finalises(workflow_producers, monkeypatch, failure):
    fixture = workflow_producers
    original = fixture["completed"]
    def completed(command, **kwargs):
        result = original(command, **kwargs)
        if command[-1] == "--version":
            if failure == "close-version" and "close-control" in command:
                result.stdout = "unexpected-close-version\n"
            elif failure == "wip-version" and "wip-tally" in command:
                result.stdout = "unexpected-wip-version\n"
            elif failure == "uv-version" and command[0] == "uv":
                result.returncode = 1
        if failure == "source-handoff" and "recurring.py" in " ".join(command):
            path = fixture["arguments"]["output"] / "lumbridge-refresh/review.json"
            document = json.loads(path.read_text())
            document["source_sha256"]["invoices.csv"] = "changed-source"
            path.write_text(json.dumps(document))
        return result
    monkeypatch.setitem(fixture["scope"], "captured", completed)
    if failure == "provenance":
        calls = 0
        def evidence(project):
            nonlocal calls
            calls += 1
            return {"revision": "a" * 40 if calls <= 4 else "b" * 40, "source": str(project)}
        monkeypatch.setitem(fixture["scope"], "project_evidence", evidence)
    with pytest.raises(ValueError) as error:
        fixture["run"](**fixture["arguments"])
    expected = {"close-version": "Native close runtime version differs", "wip-version": "Native WIP runtime version differs",
                "uv-version": "Unable to record uv version", "source-handoff": "Close and forecast sources differ",
                "provenance": "Source changed during the run"}[failure]
    assert expected in str(error.value.__cause__ or error.value)
    output = fixture["arguments"]["output"]
    assert not (output / "manifest.json").exists()
    assert "Run failed" in (output / "summary.md").read_text()
    assert (output / ("source-changes.json" if failure == "provenance" else "failed-results.json")).is_file()


def test_standalone_provenance_refuses_missing_lock_and_tracks_deleted_indexed_source(tmp_path):
    MODULE["git_output"](tmp_path, "init")
    project = tmp_path / "standalone"
    project.mkdir()
    (project / "pyproject.toml").write_text("[project]\nname='fixture'\n")
    lock = project / "uv.lock"
    lock.write_text("fabricated resolution")
    deleted = project / "deleted.py"
    deleted.write_text("fabricated = True\n")
    MODULE["git_output"](tmp_path, "add", "--", "standalone/deleted.py")
    deleted.unlink()
    evidence = MODULE["project_evidence"](project)
    assert evidence["lockfile"] == "standalone/uv.lock" and evidence["lock_scope"] == "project"
    assert "standalone/deleted.py" not in evidence["source_sha256"]
    lock.unlink()
    with pytest.raises(ValueError, match="Missing authoritative lockfile"):
        MODULE["project_evidence"](project)


def test_git_failure_reports_its_diagnostics(tmp_path, monkeypatch):
    monkeypatch.setitem(MODULE["git_output"].__globals__, "captured", lambda *args, **kwargs:
                        subprocess.CompletedProcess(args, 128, "", "fabricated Git failure"))
    with pytest.raises(ValueError, match="fabricated Git failure"):
        MODULE["git_output"](tmp_path, "status")


@pytest.mark.parametrize("failure", ["missing-grant-manifest", "missing-uv", "existing-output"])
def test_prepare_protects_external_destinations_and_requires_grant_manifest(tmp_path, monkeypatch, failure):
    fpa, grants = tmp_path / "fpa", tmp_path / "grants"
    for project in (fpa, grants):
        project.mkdir()
        (project / "pyproject.toml").write_text("[project]")
    output = tmp_path / "external-output"
    expected = {"missing-grant-manifest": "Missing project manifest", "missing-uv": "Install uv", "existing-output": "new output"}[failure]
    if failure == "missing-grant-manifest":
        (grants / "pyproject.toml").unlink()
    elif failure == "missing-uv":
        monkeypatch.setattr(MODULE["shutil"], "which", lambda name: None if name == "uv" else name)
    else:
        output.mkdir()
        (output / "sentinel").write_text("keep")
    with pytest.raises(ValueError, match=expected):
        PREPARE(output, fpa, None, grants, None, "grant-cash")
    if output.exists():
        assert (output / "sentinel").read_text() == "keep"


@pytest.mark.skipif(os.name == "nt", reason="POSIX process-group disappearance race")
def test_timeout_survives_a_process_disappearing_before_group_cleanup(tmp_path, monkeypatch):
    def disappeared(*args):
        raise ProcessLookupError("fabricated process disappeared")
    monkeypatch.setattr(MODULE["os"], "killpg", disappeared)
    with pytest.raises(subprocess.TimeoutExpired) as error:
        MODULE["captured"]([sys.executable, "-c", "import time; print('ready',flush=True); time.sleep(.05)"],
                           cwd=tmp_path, timeout=.001)
    assert "ready" in error.value.output


def test_cli_reports_verified_commands_after_real_result_validation(workflow_producers, capsys):
    fixture = workflow_producers
    arguments = [value for name, value in fixture["arguments"].items() for value in ("--" + name.replace("_", "-"), str(value))]
    assert MODULE["main"](arguments) == 0
    manifest = json.loads((fixture["arguments"]["output"] / "manifest.json").read_text())
    assert manifest["fixture_validation"] == "passed"
    assert capsys.readouterr().out == f"UTILITY WORKFLOWS VERIFIED: {len(manifest['calls'])} successful commands\n"


def test_real_cli_invalid_timeout_refuses_before_creating_output(tmp_path):
    output = tmp_path / "results"
    result = subprocess.run([sys.executable, str(ROOT / "examples/utility_workflows.py"),
                             "--fpa", str(tmp_path), "--output", str(output), "--timeout", "0"],
                            capture_output=True, text=True, timeout=10)
    assert result.returncode == 1
    assert result.stderr == "Utility workflows: Command timeout must be positive and at most 3600 seconds\n"
    assert not output.exists()
