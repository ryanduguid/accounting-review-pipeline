"""Set up and run the fabricated accounting examples in a new workspace."""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

SOURCE = Path(__file__).resolve().parent
# The pipeline checkout two levels up holds this application, the close controls and the
# joined runner, so one pipeline commit supplies both the close and the grant sources.
PIPELINE = "accounting-review-pipeline"
GRANT_PROJECT = "apps/grant-acquittal-workpapers"
DRIVER = "packages/monthly-close-control-plane/examples/utility_workflows.py"
COMPANIONS = ("au-fpa-pack", "australian-accounting")


def git_command(*arguments: str) -> list[str]:
    prefix = ["rtk", "proxy", "git"] if shutil.which("rtk") else ["git"]
    return [*prefix, *arguments]


def replay_revisions(manifest: Path) -> dict[str, str]:
    """Read only commit identities from an all-workflow provenance manifest."""
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
        if data["schema_version"] != "utility-workflows.v2" or data["workflow"] != "all":
            raise ValueError("Replay needs an all-workflow v2 manifest")
        projects = data["projects"]
        if set(projects) != {"close", "fpa", "wip", "grants"}:
            raise ValueError("Replay needs all four source revisions")
        if projects["grants"]["project"] != GRANT_PROJECT:
            raise ValueError("Replay needs a manifest from a run inside the pipeline checkout; "
                             "manifests from the separate grant repository cannot be replayed")
        result: dict[str, str] = {}
        for owner, repository, project in (
            ("close", PIPELINE, "packages/monthly-close-control-plane"),
            ("fpa", COMPANIONS[0], "."), ("wip", COMPANIONS[1], "packages/the-wip-tally"),
            ("grants", PIPELINE, GRANT_PROJECT),
        ):
            evidence = projects[owner]
            revision = evidence["revision"]
            if (evidence["project"] != project or evidence["working_tree_status"] != [] or
                    not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision)):
                raise ValueError("Replay requires clean source checkouts and full commit IDs")
            if result.setdefault(repository, revision) != revision:
                raise ValueError("Replay needs the close controls and grant workpapers from one pipeline commit")
        return result
    except (KeyError, TypeError, AttributeError) as exc:
        raise ValueError("Invalid replay provenance manifest") from exc


def checkout_revision(destination: Path, source: str, revision: str, environment: dict):
    """Fetch only the requested commit from a caller-selected fixed source."""
    subprocess.run(git_command("init", "--", str(destination)), check=True, timeout=30, env=environment)
    subprocess.run(git_command("-C", str(destination), "fetch", "--depth", "1", "--", source, revision),
                   check=True, timeout=180, env=environment)
    subprocess.run(git_command("-C", str(destination), "checkout", "--detach", "FETCH_HEAD"),
                   check=True, timeout=30, env=environment)
    actual = subprocess.run(git_command("-C", str(destination), "rev-parse", "HEAD"),
                            capture_output=True, text=True, check=True, timeout=30, env=environment)
    if actual.stdout.strip() != revision:
        raise ValueError("Fetched commit does not match the recorded revision")


def setup(workspace: Path, replay_manifest: Path | None = None) -> Path:
    if sys.version_info < (3, 14):
        raise ValueError("Setup needs Python 3.14 or later")
    for tool in ("git", "uv"):
        if not shutil.which(tool):
            raise ValueError(f"Install {tool} and put it on PATH before running setup")
    if workspace.is_symlink() or workspace.exists():
        raise ValueError("Choose a new workspace; existing paths are never reused")
    revisions = replay_revisions(replay_manifest) if replay_manifest is not None else None
    workspace = workspace.resolve()
    source = SOURCE.resolve()
    pipeline = source.parents[1]
    # Replay runs the recorded commit's runner, which is checked once that commit is fetched.
    if revisions is None and not (pipeline / DRIVER).is_file():
        raise ValueError("Run setup from apps/grant-acquittal-workpapers in an accounting-review-pipeline checkout")
    if workspace.is_relative_to(pipeline) or pipeline.is_relative_to(workspace):
        raise ValueError("The workspace must be separate from the pipeline checkout")
    ancestor = workspace.parent
    while not ancestor.exists():
        ancestor = ancestor.parent
    environment = os.environ.copy()
    environment["GIT_TERMINAL_PROMPT"] = "0"
    environment["LC_ALL"] = "C"
    probe = subprocess.run(git_command("-C", str(ancestor), "rev-parse", "--show-toplevel"),
                           capture_output=True, text=True, check=False, timeout=30, env=environment)
    if probe.returncode == 0:
        raise ValueError("The workspace must be outside existing Git checkouts")
    if probe.returncode != 128 or not probe.stderr.startswith("fatal: not a git repository"):
        raise ValueError("Git could not verify workspace isolation; fix its error before setup")
    workspace.mkdir(parents=True)
    sources = workspace / "sources"
    sources.mkdir()
    for name in COMPANIONS:
        url = f"https://github.com/ryanduguid/{name}.git"
        if revisions is None:
            subprocess.run(git_command("clone", "--depth", "1", "--branch", "main", "--single-branch",
                                       "--", url, str(sources / name)), check=True, timeout=180, env=environment)
        else:
            checkout_revision(sources / name, url, revisions[name], environment)
    if revisions is not None:
        snapshot = sources / PIPELINE
        checkout_revision(snapshot, str(pipeline), revisions[PIPELINE], environment)
        pipeline = snapshot
        if not (pipeline / DRIVER).is_file() or not (pipeline / GRANT_PROJECT).is_dir():
            raise ValueError("The recorded pipeline commit lacks the joined runner or the grant workpapers")
    output = workspace / "results"
    subprocess.run([sys.executable, str(pipeline / DRIVER), "--fpa", str(sources / "au-fpa-pack"),
                    "--accounting", str(sources / "australian-accounting"),
                    "--grants", str(pipeline / GRANT_PROJECT),
                    "--output", str(output), "--environment-root", str(workspace / "environments")], check=True)
    if not (output / "manifest.json").is_file():
        raise ValueError("The workflow returned without its success manifest")
    manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
    if not isinstance(manifest, dict) or manifest.get("fixture_validation") != "passed":
        raise ValueError("The pipeline checkout lacks the required fixture result checks")
    if revisions is not None:
        if replay_revisions(output / "manifest.json") != revisions:
            raise ValueError("Run provenance does not match the replay revisions")
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, required=True,
                        help="New directory outside every Git checkout")
    parser.add_argument("--replay-manifest", type=Path,
                        help="Replay clean source commits from an all-route manifest or replay.json")
    parser.add_argument("--summary", type=Path, help="Append a result summary outside source and workspace directories")
    args = parser.parse_args(argv)
    if args.summary is not None:
        target = args.summary.resolve()
        if any(target == root or root in target.parents
               for root in (SOURCE.resolve().parents[1], args.workspace.resolve())):
            parser.error("Summary must be outside the pipeline checkout and workspace")
    was_present = args.workspace.exists() or args.workspace.is_symlink()
    succeeded = False
    try:
        output = setup(args.workspace, args.replay_manifest)
        succeeded = True
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Setup failed: {error}. Existing checkouts were not updated. "
              "Keep any partial workspace for diagnosis and retry with a new path.", file=sys.stderr)
    result = 0 if succeeded else 1
    if args.summary is not None:
        try:
            summary = args.workspace / "results/summary.md"
            fallback = ("# Joined accounting examples\n\nFixture checks passed. Detailed summary unavailable; see results/manifest.json.\n"
                        if succeeded else
                        "# Joined accounting examples\n\nSetup failed before verified results were available. See the failed setup step and retained diagnostics.\n")
            text = summary.read_text(encoding="utf-8") if not was_present and summary.is_file() else fallback
            if not succeeded and "Fixture checks passed." in text:
                text = fallback
            with args.summary.open("a", encoding="utf-8") as stream:
                stream.write(text + "\n")
        except (OSError, UnicodeError) as error:
            print(f"Could not append the requested result summary: {error}", file=sys.stderr)
            result = 1
    if succeeded:
        print(f"Accounting examples verified. Results: {output}")
    return result


if __name__ == "__main__":
    raise SystemExit(main())
