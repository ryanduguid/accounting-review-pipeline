"""Run fabricated workflows with explicit lock authority and source evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import runpy
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

COMPONENT = Path(__file__).resolve().parents[1]
RESULTS = runpy.run_path(str(Path(__file__).with_name("utility_results.py")))


def captured(command, *, cwd, env=None, timeout=300):
    """Stop the whole command tree on timeout or interruption."""
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", errors="replace", start_new_session=os.name != "nt",
                               creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt) as exc:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True, check=False)
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        stdout, stderr = process.communicate()
        raise subprocess.TimeoutExpired(command, timeout, output=stdout, stderr=stderr) from exc
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def git_output(root, *arguments):
    command = ["git", *arguments]
    if shutil.which("rtk"):
        command = ["rtk", "proxy", *command]
    result = captured(command, cwd=root, timeout=30)
    if result.returncode:
        raise ValueError(f"Git evidence failed: {result.stderr}")
    return result.stdout


def project_evidence(project):
    import tomllib

    root = Path(git_output(project, "rev-parse", "--show-toplevel").strip()).resolve()
    relative = project.relative_to(root)
    lock_owner = project
    for parent in (project, *project.parents):
        manifest = parent / "pyproject.toml"
        if manifest.is_file():
            workspace = tomllib.loads(manifest.read_text(encoding="utf-8")).get("tool", {}).get("uv", {}).get("workspace")
            if workspace is not None:
                member = project.relative_to(parent)
                if member == Path(".") or (any(member.match(pattern) for pattern in workspace.get("members", [])) and
                                            not any(member.match(pattern) for pattern in workspace.get("exclude", []))):
                    lock_owner = parent
                    break
        if parent == root:
            break
    lock = lock_owner / "uv.lock"
    if not lock.is_file():
        raise ValueError(f"Missing authoritative lockfile: {lock}")
    paths = git_output(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")
    # Hash source and fixtures only. Never read credentials or local environment files.
    extensions = {".py", ".toml", ".lock", ".json", ".csv", ".yaml", ".yml", ".md", ".in", ".txt"}
    hashes = {}
    for name in sorted(set(paths)):
        path = Path(name)
        if (not name or path.suffix.lower() not in extensions or
                any(part.startswith(".env") or part.lower() in {"credentials", "secrets", "node_modules"} for part in path.parts)):
            continue
        full = root / path
        if full.is_symlink() or not full.is_file():
            continue
        hashes[path.as_posix()] = hashlib.sha256(full.read_bytes()).hexdigest()
    head = git_output(root, "rev-parse", "--verify", "--quiet", "HEAD") if git_output(root, "rev-list", "--all", "--count").strip() != "0" else None
    return {"repository": root.name, "project": relative.as_posix(), "revision": head.strip() if head else None,
            "working_tree_status": git_output(root, "status", "--porcelain=v1", "--untracked-files=all").splitlines(),
            "lockfile": lock.relative_to(root).as_posix(), "lock_sha256": hashlib.sha256(lock.read_bytes()).hexdigest(),
            "lock_scope": "workspace" if lock_owner != project else "project", "source_sha256": hashes,
            "source_scope": "Git-listed source and fixtures with documented text extensions; local secrets, symlinks and ignored files excluded."}


def prepare(output: Path, fpa: Path, accounting: Path | None, grants: Path | None, environment_root: Path | None, workflow: str) -> dict:
    projects = {"close": COMPONENT, "fpa": fpa.resolve()}
    if accounting is not None:
        projects["wip"] = (accounting / "packages/the-wip-tally").resolve()
    if grants is not None:
        projects["grants"] = grants.resolve()
    required = {"all": {"wip", "grants"}, "job-cash": {"wip"}, "grant-cash": {"grants"}}.get(workflow, set())
    if required - projects.keys():
        raise ValueError("This workflow needs --accounting and/or --grants checkouts")
    for project in projects.values():
        if not (project / "pyproject.toml").is_file():
            raise ValueError(f"Missing project manifest: {project}")
    output = output.resolve()
    roots = [COMPONENT.parents[1], fpa.resolve(), *([accounting.resolve()] if accounting else []), *([grants.resolve()] if grants else [])]
    roots.extend(projects.values())
    if any(output == root or root in output.parents for root in roots):
        raise ValueError("Output must be outside all source checkouts")
    if output.exists():
        raise ValueError("Choose a new output directory")
    if environment_root is not None:
        environment_root = environment_root.resolve()
        if (environment_root.exists() or environment_root == output or
                environment_root in output.parents or output in environment_root.parents):
            raise ValueError("Fresh environment root must be a separate new directory")
        if any(root == environment_root or root in environment_root.parents for root in roots):
            raise ValueError("Fresh environments must be outside source checkouts")
    if shutil.which("uv") is None:
        raise ValueError("Install uv before running the locked workflow")
    if "wip" in required and not (projects["wip"] / "examples/job_to_cash.py").is_file():
        raise ValueError("The WIP checkout needs examples/job_to_cash.py from australian-accounting PR #236")
    return {"projects": projects, "output": output, "environment_root": environment_root}


def run_workflows(*, output: Path, fpa: Path, accounting: Path | None = None, grants: Path | None = None,
                  environment_root: Path | None = None, workflow: str = "all", timeout: float = 300) -> dict:
    if not 0 < timeout <= 3600:
        raise ValueError("Command timeout must be positive and at most 3600 seconds")
    prepared = prepare(output, fpa, accounting, grants, environment_root, workflow)
    projects, output, environment_root = (prepared[key] for key in ("projects", "output", "environment_root"))
    provenance = {owner: project_evidence(project) for owner, project in projects.items()}
    output.mkdir(parents=True)
    (output / "replay.json").write_text(json.dumps({"schema_version": "utility-workflows.v2",
        "workflow": workflow, "projects": provenance}, indent=2), encoding="utf-8")
    if environment_root is not None:
        environment_root.mkdir(parents=True)
    calls = []

    def execute(owner, arguments, expected=0, kind="workflow"):
        project = projects[owner]
        command = ["uv", "run", "--project", str(project), "--locked", "--python", "3.11", "python", *map(str, arguments)]
        # RTK is optional for users. Preserve exact JSON when it is installed locally.
        if shutil.which("rtk"):
            command = ["rtk", "proxy", *command]
        env = os.environ.copy()
        if environment_root is not None:
            env["UV_PROJECT_ENVIRONMENT"] = str(environment_root / owner)
        started = time.monotonic()
        failure = None
        try:
            result = captured(command, cwd=project, env=env, timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            result = subprocess.CompletedProcess(command, None, exc.output or "", exc.stderr or "")
            failure = "timeout_or_interrupt"
        except OSError as exc:
            result = subprocess.CompletedProcess(command, None, "", str(exc))
            failure = "launch_error"
        calls.append({"owner": owner, "kind": kind, "arguments": list(map(str, arguments)), "exit_code": result.returncode,
                      "expected_exit": expected, "elapsed_seconds": round(time.monotonic() - started, 6), "failure": failure})
        if failure or result.returncode != expected:
            record = {"calls": calls, "projects": provenance, "stdout": result.stdout, "stderr": result.stderr}
            (output / "failed-calls.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
            RESULTS["summary"](output, calls, provenance, failure=f"{owner} command {len(calls)}: {failure or result.returncode}; see failed-calls.json")
            raise ValueError(f"{owner} failed ({failure or result.returncode}); see {output / 'failed-calls.json'}")
        return result.stdout

    def save(name, text):
        (output / name).write_text(text, encoding="utf-8")

    def close_pack(current, prior, subledger, target, mapping=None):
        arguments = ["-m", "closecontrol.cli", "review", "--current", current, "--prior", prior,
                     "--subledger", subledger, "--output", target]
        if mapping is not None:
            arguments += ["--mapping", mapping]
        execute("close", arguments, 2)
        execute("close", ["-m", "closecontrol.cli", "view", "--pack-dir", target])

    try:
        runtimes = {owner: json.loads(execute(owner, ["-c", "import sys,json,importlib.metadata as m; print(json.dumps({'python':sys.version,'packages':sorted((d.metadata['Name'],d.version) for d in m.distributions())}))"], kind="runtime")) for owner in projects}
        uv_version = captured(["uv", "--version"], cwd=COMPONENT, timeout=30)
        if uv_version.returncode:
            raise ValueError("Unable to record uv version")
        if workflow in {"all", "close-forecast"}:
            inputs, pack, refresh = output / "lumbridge-inputs", output / "lumbridge-close", output / "lumbridge-refresh"
            execute("fpa", ["examples/lumbridge-services/models/generated/close_handoff.py", "--output", inputs])
            close_pack(inputs / "current.csv", inputs / "prior.csv", inputs / "subledger.csv", pack, inputs / "mapping.csv")
            execute("fpa", ["examples/lumbridge-services/models/generated/recurring.py", "--output", refresh])
            handoff = json.loads((inputs / "handoff.json").read_text())
            refreshed = json.loads((refresh / "review.json").read_text())
            for name in ("xero_bs.csv", "invoices.csv", "payments.csv"):
                if handoff["source_sha256"][name] != refreshed["source_sha256"][name]:
                    raise ValueError("Close and forecast sources differ")
        if workflow in {"all", "quarter"}:
            quarter = output / "quarter"
            execute("fpa", ["examples/quarter-close/quarter.py", "--output", quarter])
            periods = json.loads((quarter / "quarter.json").read_text(encoding="utf-8"))["periods"]
            RESULTS["quarter_periods"](periods)
            for index in range(1, len(periods)):
                current, prior = (quarter / periods[i]["cutoff"] for i in (index, index - 1))
                close_pack(current / "trial-balance.csv", prior / "trial-balance.csv", current / "subledger.csv", current / "close")
                if index > 1:
                    comparison = execute("close", ["-m", "closecontrol.cli", "compare", "--previous-pack", prior / "close",
                        "--current-pack", current / "close", "--previous-tb", prior / "trial-balance.csv",
                        "--current-tb", current / "trial-balance.csv"])
                    (current / "comparison.json").write_text(comparison, encoding="utf-8")
        if workflow in {"all", "job-cash"}:
            for label, extra in (("job-base", "0"), ("job-extra-cost", "100000")):
                target = output / label
                execute("wip", ["examples/job_to_cash.py", "--output", target, "--extra-cost-to-complete", extra])
                forecast = execute("fpa", ["examples/job-to-cash/project_cash.py", "--pack", target])
                (target / "project-cash.json").write_text(forecast, encoding="utf-8")
        if workflow in {"all", "grant-cash"}:
            grant = output / "grants"
            execute("grants", ["grant_workpaper.py", "--input", "examples/two-grants", "--output", grant], 2)
            workpaper = grant / "workpaper.json"
            digest = hashlib.sha256(workpaper.read_bytes()).hexdigest()
            save("grant-cash.json", execute("fpa", ["examples/restricted-cash/grant_cash.py", "--workpaper", workpaper, "--workpaper-sha256", digest]))
        verified_lines = RESULTS["validate"](output, workflow, calls, projects)
        after = {owner: project_evidence(project) for owner, project in projects.items()}
        if after != provenance:
            (output / "source-changes.json").write_text(json.dumps({"before": provenance, "after": after}, indent=2), encoding="utf-8")
            RESULTS["summary"](output, calls, provenance, failure="Source changed during execution; see source-changes.json")
            raise ValueError("Source changed during the run; review source-changes.json")
        RESULTS["summary"](output, calls, provenance, lines=verified_lines)
        manifest = {"fixture_validation": "passed", "schema_version": "utility-workflows.v2", "workflow": workflow, "calls": calls,
                    "projects": provenance, "runtimes": runtimes, "tools": {"driver_python": sys.version, "uv": uv_version.stdout.strip()},
                    "outputs_sha256": {path.relative_to(output).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                                       for path in sorted(output.rglob("*")) if path.is_file()},
                    "scope": "Fabricated local workflows. Engine review statuses remain unchanged. Fresh environments do not establish a hosted CI result."}
        save("manifest.json", json.dumps(manifest, indent=2))
        return manifest
    except (ValueError, KeyError, TypeError, IndexError, OSError, ArithmeticError, subprocess.SubprocessError) as exc:
        (output / "manifest.json").unlink(missing_ok=True)
        if not any((output / name).exists() for name in ("failed-calls.json", "source-changes.json")):
            RESULTS["summary"](output, calls, provenance, failure="Fixture result validation failed; see failed-results.json")
            (output / "failed-results.json").write_text(json.dumps({"error": str(exc), "calls": calls,
                "projects": provenance}, indent=2), encoding="utf-8")
            raise ValueError("Fixture result validation failed; see failed-results.json") from exc
        raise



if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fpa", type=Path, required=True)
    parser.add_argument("--accounting", type=Path)
    parser.add_argument("--grants", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--environment-root", type=Path, help="optional new directory for isolated installations")
    parser.add_argument("--timeout", type=float, default=300, help="seconds per command, including setup (default: 300)")
    parser.add_argument("--workflow", choices=("all", "close-forecast", "quarter", "job-cash", "grant-cash"), default="all")
    args = parser.parse_args()
    try:
        result = run_workflows(**vars(args))
    except (ValueError, KeyError, TypeError, OSError) as exc:
        parser.exit(1, f"Utility workflows: {exc}\n")
    print(f"UTILITY WORKFLOWS VERIFIED: {len(result['calls'])} successful commands")
