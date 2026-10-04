import contextlib
import io
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import setup_utility


def manifest():
    # The close controls and the grant workpapers come from one pipeline commit.
    return {"schema_version": "utility-workflows.v2", "workflow": "all", "projects": {
        owner: {"revision": str(index) * 40, "working_tree_status": [], "project": path}
        for owner, index, path in (("close", 1, "packages/monthly-close-control-plane"),
            ("fpa", 2, "."), ("wip", 3, "packages/the-wip-tally"),
            ("grants", 1, "apps/grant-acquittal-workpapers"))}}


def fake_pipeline(root, runner=True):
    """A pipeline checkout holding the grant workpapers and, optionally, the joined runner."""
    (root / setup_utility.GRANT_PROJECT).mkdir(parents=True)
    if runner:
        driver = root / setup_utility.DRIVER
        driver.parent.mkdir(parents=True)
        driver.write_text("")


class ReplayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="recorded revision ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.manifest = self.root / "manifest.json"

    def test_manifest_uses_fixed_repository_mapping(self):
        self.manifest.write_text(json.dumps(manifest()))
        revisions = setup_utility.replay_revisions(self.manifest)
        self.assertEqual(set(revisions), {*setup_utility.COMPANIONS, "accounting-review-pipeline"})
        self.assertEqual(revisions["accounting-review-pipeline"], "1" * 40)
        self.assertEqual(revisions["au-fpa-pack"], "2" * 40)

    def test_invalid_manifests_are_refused(self):
        changes = [lambda d: d.update(workflow="quarter"), lambda d: d.update(schema_version="future"),
                   lambda d: d["projects"].pop("grants"),
                   lambda d: d["projects"]["close"].update(revision="--upload-pack=untrusted"),
                   lambda d: d["projects"]["fpa"].update(revision="main"),
                   lambda d: d["projects"]["wip"].update(working_tree_status=[" M source.py"]),
                   lambda d: d["projects"]["grants"].update(project="../../private"),
                   # A manifest from the separate grant repository.
                   lambda d: d["projects"]["grants"].update(project="."),
                   lambda d: d["projects"]["grants"].update(revision="4" * 40),
                   lambda d: d["projects"].update(close=None)]
        for change in changes:
            with self.subTest(change=change):
                data = manifest()
                change(data)
                self.manifest.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    setup_utility.replay_revisions(self.manifest)

    def test_invalid_replay_does_not_create_workspace(self):
        self.manifest.write_text("{}")
        target = self.root / "new"
        with patch.object(setup_utility.sys, "version_info", (3, 11)), patch.object(setup_utility.shutil, "which", return_value="tool"):
            with self.assertRaises(ValueError):
                setup_utility.setup(target, self.manifest)
        self.assertFalse(target.exists())

    def test_real_git_fetches_recorded_commit_without_changing_source(self):
        source = self.root / "original repo"
        source.mkdir()

        def git(*args):
            return subprocess.check_output(setup_utility.git_command("-C", str(source), *args), text=True).strip()

        git("init")
        file = source / "fixture.txt"
        file.write_text("old")
        git("add", "fixture.txt")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "commit", "-m", "first fixture")
        revision = git("rev-parse", "HEAD")
        file.write_text("new")
        git("add", "fixture.txt")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "commit", "-m", "second fixture")
        head = git("rev-parse", "HEAD")
        file.write_text("uncommitted work")
        setup_utility.checkout_revision(self.root / "snapshot", str(source), revision, os.environ.copy())
        self.assertEqual((self.root / "snapshot/fixture.txt").read_text(), "old")
        self.assertEqual(git("rev-parse", "HEAD"), head)
        self.assertEqual(file.read_text(), "uncommitted work")
        with self.assertRaises(subprocess.CalledProcessError):
            setup_utility.checkout_revision(self.root / "unavailable", str(source), "f" * 40, os.environ.copy())

    def test_checkout_rejects_unexpected_commit(self):
        with patch.object(setup_utility.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "b" * 40)):
            with self.assertRaisesRegex(ValueError, "does not match"):
                setup_utility.checkout_revision(self.root / "new", "fixed-source", "a" * 40, {})

    def test_replay_fetches_fixed_sources_and_passes_pipeline_snapshot(self):
        self.manifest.write_text(json.dumps(manifest()))
        workspace = self.root / "workspace"
        seen = []

        def checkout(destination, source, revision, environment):
            destination.mkdir()
            if destination.name == "accounting-review-pipeline":
                fake_pipeline(destination)
            seen.append((destination.name, source, revision))

        def run(args, **kwargs):
            if "rev-parse" in args:
                return subprocess.CompletedProcess(args, 128, stderr="fatal: not a git repository")
            snapshot = workspace / "sources/accounting-review-pipeline"
            self.assertEqual(args[1], str(snapshot / setup_utility.DRIVER))
            self.assertEqual(args[args.index("--grants") + 1], str(snapshot / setup_utility.GRANT_PROJECT))
            output = workspace / "results"
            output.mkdir()
            (output / "manifest.json").write_text(json.dumps({**manifest(), "fixture_validation": "passed"}))
            return subprocess.CompletedProcess(args, 0)

        with patch.object(setup_utility.sys, "version_info", (3, 11)), patch.object(setup_utility.shutil, "which", return_value="tool"), patch.object(setup_utility, "checkout_revision", side_effect=checkout), patch.object(setup_utility.subprocess, "run", side_effect=run):
            setup_utility.setup(workspace, self.manifest)
        self.assertEqual([source for _, source, _ in seen[:2]],
                         [f"https://github.com/ryanduguid/{name}.git" for name in setup_utility.COMPANIONS])
        # The pipeline commit comes from the local pipeline checkout, which may hold unpushed work.
        self.assertEqual(seen[-1], ("accounting-review-pipeline", str(setup_utility.SOURCE.resolve().parents[1]), "1" * 40))
        self.assertEqual(len(seen), 3)

    def test_replay_runner_comes_from_the_recorded_commit(self):
        self.manifest.write_text(json.dumps(manifest()))
        for recorded_runner in (True, False):
            with self.subTest(recorded_runner=recorded_runner):
                # The current checkout no longer has the runner where the recorded commit did.
                current = self.root / f"current {recorded_runner}"
                fake_pipeline(current, runner=False)
                workspace = self.root / f"workspace {recorded_runner}"
                started = []

                def checkout(destination, source, revision, environment):
                    destination.mkdir()
                    if destination.name == "accounting-review-pipeline":
                        fake_pipeline(destination, runner=recorded_runner)

                def run(args, **kwargs):
                    if "rev-parse" in args:
                        return subprocess.CompletedProcess(args, 128, stderr="fatal: not a git repository")
                    started.append(args[1])
                    output = workspace / "results"
                    output.mkdir()
                    (output / "manifest.json").write_text(json.dumps({**manifest(), "fixture_validation": "passed"}))
                    return subprocess.CompletedProcess(args, 0)

                with patch.object(setup_utility, "SOURCE", current / setup_utility.GRANT_PROJECT), patch.object(setup_utility.sys, "version_info", (3, 11)), patch.object(setup_utility.shutil, "which", return_value="tool"), patch.object(setup_utility, "checkout_revision", side_effect=checkout), patch.object(setup_utility.subprocess, "run", side_effect=run):
                    if recorded_runner:
                        setup_utility.setup(workspace, self.manifest)
                        self.assertEqual(started, [str(workspace / "sources/accounting-review-pipeline" / setup_utility.DRIVER)])
                    else:
                        with self.assertRaisesRegex(ValueError, "lacks the joined runner"):
                            setup_utility.setup(workspace, self.manifest)
                        self.assertEqual(started, [])

    def test_summary_appends_and_failed_setup_never_reads_old_results(self):
        workspace = self.root / "old workspace"
        (workspace / "results").mkdir(parents=True)
        (workspace / "results/summary.md").write_text("Fixture checks passed.")
        target = self.root / "summary.md"
        target.write_text("Earlier step\n")
        with patch.object(setup_utility.sys, "version_info", (3, 11)):
            self.assertEqual(setup_utility.main(["--workspace", str(workspace), "--summary", str(target)]), 1)
        text = target.read_text()
        self.assertTrue(text.startswith("Earlier step"))
        self.assertIn("Setup failed", text)
        self.assertNotIn("Fixture checks passed", text)

    def test_summary_success_and_command_failure(self):
        for success in (True, False):
            workspace = self.root / str(success)
            target = self.root / f"{success}.md"

            def setup(*args):
                output = workspace / "results"
                output.mkdir(parents=True)
                (output / "summary.md").write_text("Fixture checks passed." if success else "Run failed. fpa command 3.")
                if not success:
                    raise ValueError("failed")
                return output

            with patch.object(setup_utility, "setup", side_effect=setup):
                self.assertEqual(setup_utility.main(["--workspace", str(workspace), "--summary", str(target)]), 0 if success else 1)
            self.assertIn("Fixture checks passed" if success else "fpa command 3", target.read_text())

    def test_success_without_detailed_summary_has_truthful_fallback(self):
        workspace = self.root / "new"
        target = self.root / "summary.md"
        with patch.object(setup_utility, "setup", return_value=workspace / "results"):
            self.assertEqual(setup_utility.main(["--workspace", str(workspace), "--summary", str(target)]), 0)
        text = target.read_text()
        self.assertIn("Fixture checks passed", text)
        self.assertIn("Detailed summary unavailable", text)
        self.assertNotIn("Setup failed", text)

    def test_summary_write_errors_preserve_the_setup_outcome(self):
        for success in (True, False):
            for target in (self.root, self.root / "missing-parent/summary.md"):
                with self.subTest(success=success, target=target):
                    workspace = self.root / "new"
                    stdout, stderr = io.StringIO(), io.StringIO()
                    effect = None if success else ValueError("fixture setup failure")
                    with patch.object(setup_utility, "setup", side_effect=effect, return_value=workspace / "results"), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                        self.assertEqual(setup_utility.main(["--workspace", str(workspace), "--summary", str(target)]), 1)
                    self.assertIn("Could not append the requested result summary", stderr.getvalue())
                    self.assertEqual("Accounting examples verified" in stdout.getvalue(), success)
                    self.assertEqual("Setup failed" in stderr.getvalue(), not success)

    def test_unreadable_generated_summary_reports_a_controlled_error(self):
        workspace = self.root / "new"
        target = self.root / "summary.md"

        def setup(*args):
            output = workspace / "results"
            output.mkdir(parents=True)
            (output / "summary.md").write_bytes(b"\xff")
            return output

        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(setup_utility, "setup", side_effect=setup), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            self.assertEqual(setup_utility.main(["--workspace", str(workspace), "--summary", str(target)]), 1)
        self.assertIn("Accounting examples verified", stdout.getvalue())
        self.assertIn("Could not append the requested result summary", stderr.getvalue())
        self.assertFalse(target.exists())
