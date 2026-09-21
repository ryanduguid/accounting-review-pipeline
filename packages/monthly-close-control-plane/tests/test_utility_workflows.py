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
    subprocess.run(["rtk", "proxy", "git", "init", str(tmp_path)], check=True, capture_output=True)
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
    assert not (output / "manifest.json").exists()


def test_success_records_runtime_timing_and_output_hashes(tmp_path, monkeypatch):
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
    result = run(output=output, fpa=tmp_path, workflow="quarter")
    assert result["schema_version"] == "utility-workflows.v2"
    assert result["runtimes"]["fpa"]["python"] == "fixture"
    assert result["projects"]["fpa"]["revision"] == "fixture"
    assert all(row["elapsed_seconds"] >= 0 for row in result["calls"])
    assert "quarter/quarter.json" in result["outputs_sha256"]


def test_invalid_timeout_fails_before_creating_output(tmp_path):
    with pytest.raises(ValueError, match="timeout"):
        MODULE["run_workflows"](output=tmp_path / "results", fpa=tmp_path, timeout=0)
    assert not (tmp_path / "results").exists()
