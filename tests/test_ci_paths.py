"""Exercise CI path selection against actual Git trees and shared file patterns."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import subprocess
import tempfile
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


class SelectionTests(unittest.TestCase):
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
        patterns = [".github/**", "uv.lock", "contracts/xero-trial-balance-v1/**"]
        for path in (
            ".github/ci/select_paths.py",
            "uv.lock",
            "contracts/xero-trial-balance-v1/schema.json",
        ):
            self.assertTrue(select_paths.matches([path], patterns))
        self.assertFalse(select_paths.matches(["docs/note.md"], patterns))
        self.assertFalse(select_paths.matches([], patterns))

    def test_component_workflow_keeps_both_rename_paths(self) -> None:
        workflow = (ROOT / ".github/workflows/ci-package.yml").read_text(
            encoding="utf-8"
        )
        comparisons = [
            line for line in workflow.splitlines() if "changed=$(git diff" in line
        ]
        self.assertEqual(len(comparisons), 2)
        self.assertTrue(all("--no-renames" in line for line in comparisons))


if __name__ == "__main__":
    unittest.main()
