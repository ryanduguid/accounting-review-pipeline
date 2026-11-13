# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""Exercise the workflow's coverage boundaries with disposable reports."""

import json
import importlib.util
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))


def script(job, name):
    return next(step["run"] for step in WORKFLOW["jobs"][job]["steps"]
                if step.get("name") == name)


class CoverageWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "repo"
        self.component = self.root / "packages/example"
        self.component.mkdir(parents=True)

    def execute(self, source, cwd, **extra):
        env = {"GITHUB_WORKSPACE": str(self.root), "RUNNER_OS": "Native",
               "GITHUB_OUTPUT": str(self.root / "output")}
        if "SystemRoot" in os.environ:
            env["SystemRoot"] = os.environ["SystemRoot"]
        env.update(extra)
        return subprocess.run([sys.executable, "-c", source], cwd=cwd, env=env,
                              capture_output=True, text=True, timeout=10)

    def reports(self, filename):
        for profile in ("debug", "release"):
            (self.component / f"coverage-{profile}.lcov").write_text(
                f"SF:{filename}\nDA:7,3\nDA:8,0\nend_of_record\n", encoding="utf-8")
            (self.component / f"coverage-{profile}.json").write_text("{}", encoding="utf-8")

    def normalise(self):
        return self.execute(script("rust", "normalise report paths within the repository"),
                            self.component)

    def test_repository_paths_and_hit_counts_survive_both_profiles(self):
        source = self.component / "src/lib.rs"
        source.parent.mkdir()
        source.write_text("", encoding="utf-8")
        self.reports(source)
        result = self.normalise()
        self.assertEqual(result.returncode, 0, result.stderr)
        for profile in ("debug", "release"):
            name = f"coverage-example-Native-{profile}"
            self.assertEqual((self.component / (name + ".lcov")).read_text(encoding="utf-8"),
                             "SF:packages/example/src/lib.rs\nDA:7,3\nDA:8,0\nend_of_record\n")
            self.assertEqual((self.component / (name + ".json")).read_text(), "{}")

    def test_evatt_reports_require_child_refusal_counters_in_both_profiles(self):
        self.component = self.root / "packages/evatt"
        self.component.mkdir()
        source = self.component / "src/main.rs"
        source.parent.mkdir()
        source.write_text("fn run() -> i32 {\n"
                          "    if let Err(message) = stdio::configure() {\n"
                          "        return 1;\n    }\n    0\n}\n", encoding="utf-8")
        parent = self.component / "tests/cli.rs"
        for debug, release in ((0, 0), (1, 0), (0, 1), (1, 1)):
            with self.subTest(debug=debug, release=release):
                for profile, count in (("debug", debug), ("release", release)):
                    (self.component / f"coverage-{profile}.lcov").write_text(
                        f"SF:{source}\nDA:3,{count}\nend_of_record\n"
                        f"SF:{parent}\nDA:3,7\nend_of_record\n", encoding="utf-8")
                    (self.component / f"coverage-{profile}.json").write_text("{}", encoding="utf-8")
                result = self.normalise()
                if debug and release:
                    self.assertEqual(result.returncode, 0, result.stderr)
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("Evatt child stream-refusal counters are missing", result.stderr)

    def test_similarly_named_sibling_is_outside_repository(self):
        self.reports(self.root.with_name("repo-other") / "src/lib.rs")
        result = self.normalise()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not in the subpath", result.stderr)

    def test_relative_source_is_refused(self):
        self.reports("src/lib.rs")
        result = self.normalise()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Non-absolute coverage path", result.stderr)

    def test_empty_report_is_refused(self):
        self.reports(self.component / "src/lib.rs")
        (self.component / "coverage-debug.lcov").write_text("", encoding="utf-8")
        result = self.normalise()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No source files", result.stderr)

    def inventory(self, names):
        directory = self.root / "coverage"
        directory.mkdir()
        for name in names:
            (directory / name).write_text("", encoding="utf-8")
        return self.execute(script("upload-coverage", "require every current component and platform report"),
                            self.root)

    def expected_reports(self):
        components = ("elizabeth-anne-alexander", "evatt", "xero-trial-balance-export",
                      "review-ready-gate", "monthly-close-control-plane",
                      "accounting-excel-toolkit", "grant-acquittal-workpapers",
                      "australian-accounting-power-bi")
        return {f"coverage-{component}-{platform}-{profile}.lcov"
                for component in components for platform in ("Linux", "Windows")
                for profile in ("debug", "release")} | {f"coverage-python-{platform}.xml"
                                                   for platform in ("Linux", "Windows")}

    def test_inventory_supplies_only_the_complete_current_report_set(self):
        names = self.expected_reports()
        result = self.inventory(names)
        self.assertEqual(result.returncode, 0, result.stderr)
        output = (self.root / "output").read_text(encoding="utf-8").strip()
        self.assertEqual(output, "files=" + ",".join("coverage/" + name for name in sorted(names)))

    def test_missing_report_is_refused(self):
        names = self.expected_reports()
        names.remove("coverage-evatt-Windows-release.lcov")
        result = self.inventory(names)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing=['coverage-evatt-Windows-release.lcov']", result.stderr)

    def test_unexpected_report_is_refused(self):
        result = self.inventory(self.expected_reports() | {"coverage-retired-Linux-debug.lcov"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("extra=['coverage-retired-Linux-debug.lcov']", result.stderr)

    def test_missing_power_bi_report_is_refused(self):
        names = self.expected_reports()
        names.remove("coverage-australian-accounting-power-bi-Windows-release.lcov")
        result = self.inventory(names)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing=['coverage-australian-accounting-power-bi-Windows-release.lcov']", result.stderr)

    def test_only_a_fork_may_skip_the_uploader(self):
        gate = script("pipeline-gates", "Require the selected checks to complete")
        for required, uploader, rust, expected in (
            ("true", "success", "success", 0),
            ("true", "skipped", "success", 1),
            ("false", "skipped", "success", 0),
            ("false", "failure", "success", 1),
            ("false", "skipped", "skipped", 1),
        ):
            with self.subTest(required=required, uploader=uploader, rust=rust):
                results = {"rust": {"result": rust}, "upload-coverage": {"result": uploader}}
                result = self.execute(gate, self.root, RESULTS=json.dumps(results),
                                      COVERAGE_REQUIRED=required)
                self.assertEqual(result.returncode, expected, result.stderr)

    def test_python_report_seeds_unexecuted_native_paths_once(self):
        first, second = self.component / "first.py", self.component / "second.py"
        first.write_text("executed = 1\n", encoding="utf-8")
        second.write_text("unexecuted = 2\nalso_unexecuted = 3\n", encoding="utf-8")
        config = self.root / "coverage.ini"
        config.write_text("[run]\nrelative_files = true\ninclude =\n"
                          + f"    {first}\n    {second}\n", encoding="utf-8")
        env = os.environ | {"COVERAGE_FILE": str(self.root / ".coverage"),
                            "COVERAGE_RCFILE": str(config)}
        measured = subprocess.run([sys.executable, "-m", "coverage", "run", str(first)],
                                  cwd=self.root, env=env, capture_output=True, text=True, timeout=10)
        self.assertEqual(measured.returncode, 0, measured.stderr)
        report = script("windows-workspace", "retain every maintained Python source in the report")
        source = report.split("<<'PY'\n", 1)[1].rsplit("\nPY", 1)[0]
        result = self.execute(source, self.root, COVERAGE_RCFILE=str(config),
                              COVERAGE_FILE=str(self.root / ".coverage"))
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads((self.root / "coverage-python-Native.json").read_text(encoding="utf-8"))
        self.assertEqual({Path(name).as_posix() for name in data["files"]}, {"packages/example/first.py", "packages/example/second.py"})
        self.assertEqual(len(data["files"]), 2)
        self.assertEqual(data["totals"]["num_statements"], 3)
        self.assertEqual(data["totals"]["covered_lines"], 1)
        self.assertEqual(data["totals"]["missing_lines"], 2)

    def test_differential_child_keeps_only_explicit_profile_configuration(self):
        sources = (
            "packages/elizabeth-anne-alexander/tests/differential/differential.py",
            "apps/grant-acquittal-workpapers/tests/differential/differential.py",
        )
        profile = str(self.root / "profiles/default_%m_%p.profraw")
        child_profile = profile.replace(".profraw", "%c.profraw")
        for index, relative in enumerate(sources):
            with self.subTest(source=relative):
                name = f"coverage_environment_contract_{index}"
                spec = importlib.util.spec_from_file_location(name, ROOT / relative)
                module = importlib.util.module_from_spec(spec)
                sys.modules[name] = module
                self.addCleanup(sys.modules.pop, name, None)
                spec.loader.exec_module(module)
                for variables, expected in (
                    ({"LLVM_PROFILE_FILE": profile}, profile),
                    ({"LLVM_PROFILE_FILE": profile,
                      "ARP_CHILD_LLVM_PROFILE_FILE": child_profile}, child_profile),
                    ({"ARP_CHILD_LLVM_PROFILE_FILE": child_profile}, child_profile),
                ):
                    with patch.dict(os.environ, variables, clear=True):
                        self.assertEqual(module.child_env().get("LLVM_PROFILE_FILE"), expected)
                        self.assertEqual(module.child_env(LLVM_PROFILE_FILE="explicit")["LLVM_PROFILE_FILE"], "explicit")
                with patch.dict(os.environ, {"UNRELATED_PROFILE_SENTINEL": "fabricated"}, clear=True):
                    self.assertNotIn("LLVM_PROFILE_FILE", module.child_env())
                    self.assertNotIn("UNRELATED_PROFILE_SENTINEL", module.child_env())

    def test_test_harness_keeps_the_original_profile_for_native_children(self):
        profile = (self.root / "profiles/profile-%p-%m%c.profraw").as_posix()
        (self.root / "coverage-env.sh").write_text(
            "export LLVM_PROFILE_FILE=" + shlex.quote(profile) + "\n", encoding="utf-8"
        )
        bash = shutil.which("bash")
        if os.name == "nt":
            git = Path(shutil.which("git"))
            bash = next(str(parent / "bin/bash.exe") for parent in git.parents
                        if (parent / "bin/bash.exe").is_file())
        fake_cargo = """cargo() {
  "$PROFILE_TEST_PYTHON" -c 'import json,os,sys; print(json.dumps({"parent": os.environ.get("LLVM_PROFILE_FILE"), "child": os.environ.get("ARP_CHILD_LLVM_PROFILE_FILE"), "args": sys.argv[1:]}))' "$@"
}
"""
        result = subprocess.run(
            [bash, "--noprofile", "--norc", "-e", "-o", "pipefail", "-c",
             fake_cargo + script("rust", "test with line coverage")],
            cwd=self.component,
            env=os.environ | {"RUNNER_TEMP": self.root.as_posix(),
                              "PROFILE_TEST_PYTHON": Path(sys.executable).as_posix(),
                              "ARP_CHILD_LLVM_PROFILE_FILE": "stale-profile"},
            capture_output=True, text=True, timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        results = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(results, [
            {"parent": profile.replace("%c", ""), "child": profile,
             "args": ["test", "--locked", "--all-features"] + release}
            for release in ([], ["--release"])
        ])

    def test_windows_normal_returning_binaries_flush_without_continuous_counters(self):
        bash = shutil.which("bash")
        if os.name == "nt":
            git = Path(shutil.which("git"))
            bash = next(str(parent / "bin/bash.exe") for parent in git.parents
                        if (parent / "bin/bash.exe").is_file())
        profile = (self.root / "profiles/profile-%p-%20m.profraw").as_posix()
        fake_cargo = """cargo() {
  case "$*" in
    'llvm-cov show-env --sh') printf 'export LLVM_PROFILE_FILE=%q\\n' "$PROFILE_TEST_PATTERN" ;;
    'llvm-cov clean --workspace') ;;
    *) return 2 ;;
  esac
}
"""
        inspect = """
source "$RUNNER_TEMP/coverage-env.sh"
"$PROFILE_TEST_PYTHON" -c 'import json,os; print(json.dumps({name: os.environ.get(name) for name in ("LLVM_PROFILE_FILE", "ARP_CHILD_LLVM_PROFILE_FILE", "RUSTFLAGS")}))'
"""
        for component, continuous in (("apps/australian-accounting-power-bi", False),
                                      ("packages/evatt", False),
                                      ("packages/review-ready-gate", True)):
            with self.subTest(component=component):
                env = os.environ | {
                    "RUNNER_OS": "Windows", "PACKAGE_DIR": component,
                    "RUNNER_TEMP": self.root.as_posix(),
                    "PROFILE_TEST_PATTERN": profile,
                    "PROFILE_TEST_PYTHON": Path(sys.executable).as_posix(),
                    "RUSTFLAGS": "-Dwarnings",
                }
                env.pop("CARGO_ENCODED_RUSTFLAGS", None)
                result = subprocess.run(
                    [bash, "--noprofile", "--norc", "-e", "-o", "pipefail", "-c",
                     fake_cargo + script("rust", "prepare coverage for tests and external commands") + inspect],
                    cwd=self.component, env=env, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                actual = json.loads(result.stdout)
                expected = profile.replace("%20m", "%m").replace(".profraw", "%c.profraw") if continuous else profile
                self.assertEqual(actual["LLVM_PROFILE_FILE"], expected)
                self.assertEqual(actual["ARP_CHILD_LLVM_PROFILE_FILE"], expected)
                self.assertEqual("-runtime-counter-relocation" in actual["RUSTFLAGS"], continuous)


if __name__ == "__main__":
    unittest.main()
