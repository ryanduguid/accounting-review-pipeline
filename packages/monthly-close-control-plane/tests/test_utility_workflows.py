"""The portable runner refuses destructive destinations before running commands."""
import json
import runpy
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


@pytest.mark.skipif(sys.version_info < (3, 11), reason="The cross-repository driver requires Python 3.11")
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


def test_git_evidence_does_not_require_optional_rtk(tmp_path, monkeypatch):
    scope = MODULE["git_output"].__globals__
    commands = []
    monkeypatch.setattr(scope["shutil"], "which", lambda name: None)

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


@pytest.mark.parametrize("failure", ["runtime", "handoff", "workpaper", "hash", "manifest"])
def test_output_and_finalisation_failures_retain_failure_summary(tmp_path, monkeypatch, failure):
    run = MODULE["run_workflows"]
    scope = run.__globals__
    output = tmp_path / "results"
    projects = {name: tmp_path for name in ("fpa", "close", "grants")}
    monkeypatch.setitem(scope, "prepare", lambda *args: {"projects": projects, "output": output, "environment_root": None})
    monkeypatch.setitem(scope, "project_evidence", lambda project: {"revision": "fixture"})

    def completed(command, **kwargs):
        if command[-1] == "--version":
            return subprocess.CompletedProcess(command, 0, "uv fixture", "")
        if "-c" in command:
            return subprocess.CompletedProcess(command, 0, "invalid JSON" if failure == "runtime" else "{}", "")
        if "grant_workpaper.py" in command:
            if failure != "workpaper":
                target = output / "grants"
                target.mkdir()
                (target / "workpaper.json").write_text("{}")
            return subprocess.CompletedProcess(command, 2, "", "")
        return subprocess.CompletedProcess(command, 2 if "review" in command else 0, "{}", "")

    monkeypatch.setitem(scope, "captured", completed)
    if failure in {"hash", "manifest"}:
        # Isolate finalisation faults after validation, without replacing filesystem behaviour elsewhere.
        monkeypatch.setitem(scope["RESULTS"], "validate", lambda *args: ["Verified fixture"])
        original_read = Path.read_bytes
        original_write = Path.write_text

        def read(path):
            if failure == "hash" and path == output / "summary.md":
                raise OSError("fixture hash read failure")
            return original_read(path)

        def write(path, data, *args, **kwargs):
            if failure == "manifest" and path == output / "manifest.json":
                original_write(path, "partial manifest")
                raise OSError("fixture manifest write failure")
            return original_write(path, data, *args, **kwargs)

        monkeypatch.setattr(Path, "read_bytes", read)
        monkeypatch.setattr(Path, "write_text", write)
    workflow = "close-forecast" if failure == "handoff" else "grant-cash"
    with pytest.raises(ValueError, match="Fixture result validation failed"):
        run(output=output, fpa=tmp_path, grants=tmp_path, workflow=workflow)
    assert not (output / "manifest.json").exists()
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
