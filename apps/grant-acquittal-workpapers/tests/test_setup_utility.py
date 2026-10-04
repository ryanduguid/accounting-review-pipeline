import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import setup_utility


class SetupUtilityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="utility setup ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.pipeline = self.root / "pipeline checkout"
        self.source = self.pipeline / setup_utility.GRANT_PROJECT
        self.source.mkdir(parents=True)
        self.driver = self.pipeline / setup_utility.DRIVER
        self.driver.parent.mkdir(parents=True)
        self.driver.write_text("")
        self.workspace = self.root / "new workspace"
        self.calls = []
        self.addCleanup(patch.stopall)
        patch.object(setup_utility, "SOURCE", self.source).start()
        patch.object(setup_utility.sys, "version_info", (3, 14)).start()
        patch.object(setup_utility.shutil, "which", side_effect=lambda name: None if name == "rtk" else name).start()

    def command(self, arguments, **kwargs):
        self.calls.append((arguments, kwargs))
        if "rev-parse" in arguments:
            return subprocess.CompletedProcess(arguments, 128, stderr="fatal: not a git repository (or any of the parent directories): .git\n")
        if "clone" in arguments:
            Path(arguments[-1]).mkdir()
        else:
            output = Path(arguments[arguments.index("--output") + 1])
            output.mkdir()
            (output / "manifest.json").write_text(json.dumps({"fixture_validation": "passed"}))
        return subprocess.CompletedProcess(arguments, 0)

    def test_success_uses_fixed_public_main_and_separate_paths_with_spaces(self):
        with patch.object(setup_utility.subprocess, "run", side_effect=self.command):
            output = setup_utility.setup(self.workspace)
        self.assertEqual(output, self.workspace / "results")
        clones = [args for args, _ in self.calls if "clone" in args]
        self.assertEqual(len(clones), 2)
        for name, args in zip(setup_utility.COMPANIONS, clones):
            self.assertIn(f"https://github.com/ryanduguid/{name}.git", args)
            self.assertEqual(args[args.index("--branch") + 1], "main")
        driver, options = self.calls[-1]
        # The enclosing pipeline checkout runs the close controls and supplies the grant route.
        self.assertEqual(driver[1], str(self.driver))
        self.assertEqual(driver[driver.index("--grants") + 1], str(self.source))
        self.assertEqual(driver[driver.index("--environment-root") + 1], str(self.workspace / "environments"))
        self.assertTrue(options["check"])
        self.assertEqual(list(self.source.iterdir()), [])

    def test_existing_workspace_is_preserved_before_commands(self):
        self.workspace.mkdir()
        sentinel = self.workspace / "keep.txt"
        sentinel.write_text("original")
        with patch.object(setup_utility.subprocess, "run") as command:
            with self.assertRaisesRegex(ValueError, "existing"):
                setup_utility.setup(self.workspace)
        command.assert_not_called()
        self.assertEqual(sentinel.read_text(), "original")

    def test_source_nested_workspace_is_refused(self):
        for target in (self.source / "outputs", self.pipeline / "outputs"):
            with self.subTest(target=target), self.assertRaisesRegex(ValueError, "separate"):
                setup_utility.setup(target)
        self.assertEqual(list(self.source.iterdir()), [])
        self.assertFalse((self.pipeline / "outputs").exists())

    def test_application_outside_a_pipeline_checkout_is_refused(self):
        self.driver.unlink()
        with patch.object(setup_utility.subprocess, "run") as command:
            with self.assertRaisesRegex(ValueError, "accounting-review-pipeline checkout"):
                setup_utility.setup(self.workspace)
        command.assert_not_called()
        self.assertFalse(self.workspace.exists())

    def test_other_git_checkout_is_refused_before_creating_workspace(self):
        with patch.object(setup_utility.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)):
            with self.assertRaisesRegex(ValueError, "Git checkouts"):
                setup_utility.setup(self.workspace)
        self.assertFalse(self.workspace.exists())

    def test_dangling_workspace_link_is_refused(self):
        target = self.root / "missing target"
        try:
            self.workspace.symlink_to(target, target_is_directory=True)
        except OSError:
            self.skipTest("This host does not permit directory symlinks")
        with self.assertRaisesRegex(ValueError, "existing"):
            setup_utility.setup(self.workspace)
        self.assertFalse(target.exists())

    def test_git_probe_errors_do_not_bypass_workspace_isolation(self):
        for index, message in enumerate(("fatal: detected dubious ownership in repository", "fatal: Permission denied", "")):
            with self.subTest(message=message):
                target = self.root / f"probe failure {index}"
                def probe_error(arguments, **kwargs):
                    if "rev-parse" in arguments:
                        return subprocess.CompletedProcess(arguments, 128, stderr=message)
                    return self.command(arguments, **kwargs)
                with patch.object(setup_utility.subprocess, "run", side_effect=probe_error):
                    with self.assertRaisesRegex(ValueError, "verify workspace isolation"):
                        setup_utility.setup(target)
                self.assertFalse(target.exists())

    def test_missing_tools_do_not_create_workspace(self):
        for missing in ("git", "uv"):
            with self.subTest(tool=missing), patch.object(setup_utility.shutil, "which", side_effect=lambda name: None if name == missing else name):
                with self.assertRaisesRegex(ValueError, missing):
                    setup_utility.setup(self.workspace)
        self.assertFalse(self.workspace.exists())

    def test_old_python_fails_before_workspace_creation(self):
        with patch.object(setup_utility.sys, "version_info", (3, 13)):
            with self.assertRaisesRegex(ValueError, "3.14"):
                setup_utility.setup(self.workspace)
        self.assertFalse(self.workspace.exists())

    def test_failed_clone_stops_without_running_workflows(self):
        def fail_clone(arguments, **kwargs):
            if "clone" in arguments:
                raise subprocess.CalledProcessError(1, arguments)
            return self.command(arguments, **kwargs)
        with patch.object(setup_utility.subprocess, "run", side_effect=fail_clone):
            self.assertEqual(setup_utility.main(["--workspace", str(self.workspace)]), 1)
        self.assertFalse((self.workspace / "results").exists())
        self.assertEqual(list(self.source.iterdir()), [])

    def test_workflow_failure_is_not_reported_as_success(self):
        def fail_workflow(arguments, **kwargs):
            if "--output" in arguments:
                raise subprocess.CalledProcessError(2, arguments)
            return self.command(arguments, **kwargs)
        with patch.object(setup_utility.subprocess, "run", side_effect=fail_workflow):
            self.assertEqual(setup_utility.main(["--workspace", str(self.workspace)]), 1)

    def test_missing_manifest_is_not_reported_as_success(self):
        def no_outputs(arguments, **kwargs):
            return subprocess.CompletedProcess(arguments, 128 if "rev-parse" in arguments else 0,
                                               stderr="fatal: not a git repository (or any of the parent directories): .git\n")
        with patch.object(setup_utility.subprocess, "run", side_effect=no_outputs):
            with self.assertRaisesRegex(ValueError, "success manifest"):
                setup_utility.setup(self.workspace)

    def test_git_uses_optional_rtk_without_requiring_it(self):
        self.assertEqual(setup_utility.git_command("status"), ["git", "status"])
        with patch.object(setup_utility.shutil, "which", return_value="rtk"):
            self.assertEqual(setup_utility.git_command("status"), ["rtk", "proxy", "git", "status"])
