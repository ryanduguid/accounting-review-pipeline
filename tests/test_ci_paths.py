# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""Exercise CI path selection against actual Git trees and shared file patterns."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "select_paths", ROOT / ".github/ci/select_paths.py"
)
assert SPEC is not None and SPEC.loader is not None
select_paths = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(select_paths)


def configured_ci_paths(workflow: str) -> list[str]:
    """The CI_PATHS block the named workflow passes to select_paths."""
    text = (ROOT / ".github" / "workflows" / workflow).read_text(encoding="utf-8")
    match = re.search(r"(?m)^(\s+)CI_PATHS: \|\n((?:\1  .*\n)+)", text)
    assert match is not None, f"{workflow} declares no CI_PATHS block"
    return [line.strip() for line in match.group(2).splitlines() if line.strip()]


class NativeAuditTests(unittest.TestCase):
    def run_audit(self, locks: list[str], *, failed_lock: str = "", broken_git: bool = False):
        """Run the workflow's real shell step against a disposable tracked tree."""
        text = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        job = re.search(r"(?ms)^  dependency-audit:\n(.*?)(?=^  \S|\Z)", text)
        self.assertIsNotNone(job)
        step = re.search(r"(?m)^        run: (.+)\n((?:          .*\n)*)", job[1])
        self.assertIsNotNone(step)
        script = textwrap.dedent(step[2]) if step[1] == "|" else step[1]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "--quiet", directory], check=True)
            for name in locks:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("# fabricated lock fixture\n", encoding="utf-8")
                subprocess.run(["git", "-C", directory, "add", "--", name], check=True)
            untracked = root / "untracked/Cargo.lock"
            untracked.parent.mkdir()
            untracked.write_text("# not a maintained lock\n", encoding="utf-8")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            cargo = bin_dir / "cargo"
            cargo.write_text(
                '#!/usr/bin/env bash\nfile=Cargo.lock\n'
                'while (($#)); do\n'
                '  if [[ "$1" == --file ]]; then file=$2; shift; fi\n'
                '  shift\ndone\n'
                'printf "%s\\0" "$file" >> "$AUDIT_LOG"\n'
                '[[ -f "$file" && "$file" != "$FAIL_LOCK" ]]\n',
                encoding="utf-8", newline="\n",
            )
            cargo.chmod(0o755)
            if broken_git:
                # A function, not a PATH entry: Git Bash on Windows puts its own git first on PATH.
                script = 'git() { printf "packages/first/Cargo.lock\\0"; return 42; }\n' + script
            log = root / "audit.log"
            env = {
                **os.environ,
                "PATH": str(bin_dir) + os.pathsep + os.environ["PATH"],
                "RUNNER_TEMP": root.as_posix(),
                "AUDIT_LOG": log.as_posix(),
                "FAIL_LOCK": failed_lock,
            }
            working = re.search(r"(?m)^        working-directory: (.+)$", job[1])
            cwd = root / working[1] if working else root
            cwd.mkdir(parents=True, exist_ok=True)
            result = subprocess.run(
                [shutil.which("bash"), "-eo", "pipefail", "-c", script],
                cwd=cwd, env=env, capture_output=True, text=True,
                timeout=30,
            )
            audited = log.read_bytes().decode().split("\0")[:-1] if log.exists() else []
            return result, audited

    def test_audits_all_tracked_locks_including_fuzz_and_new_components(self):
        inventory = subprocess.run(
            ["git", "ls-files", "-z", "--", "Cargo.lock", "**/Cargo.lock"],
            cwd=ROOT, capture_output=True, check=True, timeout=30,
        )
        locks = inventory.stdout.decode("utf-8").split("\0")[:-1]
        self.assertTrue(locks)
        locks.append("packages/new component/Cargo.lock")
        result, audited = self.run_audit(locks)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertCountEqual(audited, locks)

    def test_one_failed_audit_fails_job_and_keeps_checking_other_locks(self):
        locks = ["packages/first/Cargo.lock", "packages/second/Cargo.lock"]
        result, audited = self.run_audit(locks, failed_lock=locks[0])
        self.assertNotEqual(result.returncode, 0)
        self.assertCountEqual(audited, locks)

    def test_missing_inventory_or_failed_git_cannot_pass(self):
        for broken_git in (False, True):
            with self.subTest(broken_git=broken_git):
                locks = ["packages/first/Cargo.lock"] if broken_git else []
                result, audited = self.run_audit(locks, broken_git=broken_git)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(audited, [])


class SelectionTests(unittest.TestCase):
    def test_cli_with_no_known_base_requires_the_checks(self) -> None:
        environment = {**os.environ, "CI_BASE": "", "CI_PATHS": "packages/evatt/**"}
        result = subprocess.run([sys.executable, str(ROOT / ".github/ci/select_paths.py")],
                                env=environment, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "run=true\n")

    def test_unknown_base_runs_checks(self) -> None:
        for base in ("", "0" * 40, "missing-commit"):
            with (
                self.subTest(base=base),
                patch.dict(
                    select_paths.os.environ,
                    {"CI_BASE": base, "CI_PATHS": "packages/evatt/**"},
                ),
                patch.object(
                    select_paths.subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess([], 1),
                ),
                contextlib.redirect_stdout(io.StringIO()) as output,
            ):
                select_paths.main()
                self.assertEqual(output.getvalue(), "run=true\n")

    def test_failed_diff_does_not_become_a_skip(self) -> None:
        with (
            patch.dict(
                select_paths.os.environ,
                {"CI_BASE": "known", "CI_PATHS": "packages/evatt/**"},
            ),
            patch.object(
                select_paths.subprocess,
                "run",
                side_effect=[
                    subprocess.CompletedProcess([], 0),
                    subprocess.CalledProcessError(1, "git diff"),
                ],
            ),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            select_paths.main()

    def test_missing_patterns_fail(self) -> None:
        with (
            patch.dict(select_paths.os.environ, {"CI_PATHS": ""}),
            self.assertRaises(ValueError),
        ):
            select_paths.main()

    def test_move_selects_both_components(self) -> None:
        with tempfile.TemporaryDirectory() as directory:

            def git(*args: str, text: str | None = None) -> str:
                return subprocess.run(
                    ["git", "-C", directory, *args],
                    input=text,
                    text=True,
                    capture_output=True,
                    check=True,
                ).stdout.strip()

            git("init", "--quiet")
            blob = git("hash-object", "-w", "--stdin", text="synthetic fixture\n")
            source = "packages/review-ready-gate/example.py"
            destination = "packages/evatt/example.py"
            git("update-index", "--add", "--cacheinfo", f"100644,{blob},{source}")
            before = git("write-tree")
            git("update-index", "--force-remove", source)
            git("update-index", "--add", "--cacheinfo", f"100644,{blob},{destination}")
            after = git("write-tree")
            # The old command loses the source path when Git detects the rename.
            old = git("-c", "diff.renames=true", "diff", "--name-only", before, after)
            self.assertNotIn(source, old.splitlines())
            real_run = subprocess.run
            with patch.object(
                select_paths.subprocess,
                "run",
                side_effect=lambda *a, **kw: real_run(*a, cwd=directory, **kw),
            ):
                paths = select_paths.changed_paths(before, after)
            self.assertEqual(set(paths), {source, destination})
            for prefix in ("packages/review-ready-gate/**", "packages/evatt/**"):
                self.assertTrue(select_paths.matches(paths, [prefix]))

    def test_shared_paths_run_and_unrelated_paths_skip(self) -> None:
        # The workflow's own CI_PATHS, not a copy of it: a local list stayed green
        # when the workflow dropped a pattern, and CI then skipped those checks.
        patterns = configured_ci_paths("joined-conformance.yml")
        for required in (".github/**", "uv.lock", "contracts/xero-trial-balance-v1/**"):
            self.assertIn(required, patterns)
        for path in (
            ".github/ci/select_paths.py",
            "uv.lock",
            "contracts/xero-trial-balance-v1/schema.json",
        ):
            self.assertTrue(select_paths.matches([path], patterns))
        self.assertFalse(select_paths.matches(["docs/note.md"], patterns))
        self.assertFalse(select_paths.matches([], patterns))

    def test_gated_workflows_select_their_former_trigger_paths(self) -> None:
        # These workflows dropped on.pull_request.paths for a changes job, so their
        # CI_PATHS must still select every path the trigger filter selected.
        for workflow, paths in (
            ("devcontainer.yml", (
                ".devcontainer/devcontainer.json",
                ".github/workflows/devcontainer.yml",
                "uv.lock",
            )),
            ("reviewer-quick-start.yml", (
                "packages/monthly-close-control-plane/docs/reviewer-quick-start.md",
                ".github/workflows/reviewer-quick-start.yml",
            )),
        ):
            with self.subTest(workflow=workflow):
                patterns = configured_ci_paths(workflow)
                for path in paths:
                    self.assertTrue(select_paths.matches([path], patterns), path)
                self.assertFalse(select_paths.matches(["docs/note.md"], patterns))


if __name__ == "__main__":
    unittest.main()
