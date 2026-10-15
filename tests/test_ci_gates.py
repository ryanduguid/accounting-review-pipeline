"""Exercise the current Python gate CLI with isolated Git workflow fixtures.

Adapted from Ozzit's test_check_gates.py real-parent regression. This tests the
existing Python CLI, so its fixture and assertions use the repository's Python.
"""

import os
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]

OLD_GATE = "pipeline-gates"
NEEDS = "    needs: [component, test, dependency-audit, package, lint, windows, rust, windows-workspace]\n"
SPEC = importlib.util.spec_from_file_location("check_gates", ROOT / ".github/ci/check_gates.py")
assert SPEC is not None and SPEC.loader is not None
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def workflow(jobs):
    return "name: fixture\n\non: pull_request\n\njobs:\n" + jobs


class GateReaderTests(unittest.TestCase):
    def names(self, jobs):
        parsed = checker.parse_jobs(workflow(jobs))
        self.assertIsNotNone(parsed)
        self.assertEqual(parsed[1], [])
        return parsed[0]

    def base(self, sources, head="  replacement:\n    name: protected\n"):
        def git(*args):
            if args[0] == "ls-tree":
                return "" if "-r" not in args else "\0".join(sources) + "\0"
            return sources[args[1].split(":", 1)[1]]
        with patch.object(checker, "git", side_effect=git):
            return checker.base_jobs(".github/workflows/new.yml", "replacement", workflow(head))

    def test_literal_names_and_quoted_job_keys(self):
        self.assertEqual(self.names("  'plain':\n    runs-on: linux\n  other:\n    name: 'It''s quoted' # comment\n"), {"plain": {"plain"}, "other": {"It's quoted"}})

    def test_static_include_resolves_only_its_complete_rows(self):
        body = "  build:\n    name: ${{ matrix.package_dir }}\n    strategy:\n      matrix:\n        include:\n          - package_dir: first\n            import_name: first_import\n          - package_dir: 'second'\n            import_name: second_import\n"
        self.assertEqual(self.names(body)["build"], {"first", "second"})
        for replacement in ("            import_name: first_import", "          - package_dir: ${{ inputs.package }}", "          - package_dir: *alias", "          - package_dir: first\n            package_dir: second"):
            with self.subTest(replacement=replacement):
                names = self.names(body.replace("          - package_dir: first", replacement))["build"]
                self.assertTrue(checker.possible_context(names, "protected"))

    def test_extra_matrix_dimension_is_unknown(self):
        jobs = "  build:\n    name: ${{ matrix.package_dir }}\n    strategy:\n      matrix:\n        os: [linux]\n        include:\n          - package_dir: first\n"
        self.assertTrue(checker.possible_context(self.names(jobs)["build"], "protected"))

    def test_typed_yaml_matrix_scalars_are_not_misread_as_strings(self):
        for value in ("0x10", "1_000", "2026-10-15", ".nan", "YES", "on"):
            with self.subTest(value=value):
                jobs = "  build:\n    name: ${{ matrix.label }}\n    strategy:\n      matrix:\n        include:\n          - label: " + value + "\n"
                self.assertTrue(checker.possible_context(self.names(jobs)["build"], "protected"))

    def test_conditional_has_both_literal_outcomes(self):
        for condition in ("linux", "other"):
            names = self.names("  check:\n    name: ${{ matrix.os == '" + condition + "' && 'first' || 'second' }}\n")["check"]
            self.assertEqual(names, {"first", "second"})
            self.assertFalse(checker.possible_context(names, "protected"))

    def test_unknown_conditionals_do_not_disappear(self):
        for expression in ("matrix.os && '' || 'second'", "needs.build.outputs.name", "matrix.os == 'linux' && inputs.name || 'second'"):
            with self.subTest(expression=expression):
                names = self.names("  check:\n    name: ${{ " + expression + " }}\n")["check"]
                self.assertTrue(checker.possible_context(names, "protected"))

    def test_unknown_templates_keep_literal_ends(self):
        names = self.names("  check:\n    name: prefix (${{ inputs.name }})\n")["check"]
        self.assertFalse(checker.possible_context(names, "protected"))
        self.assertTrue(checker.possible_context(names, "prefix (protected)"))

    def test_expression_literal_ends_cannot_overlap(self):
        for pattern, context, expected in (
            ("new-${{ matrix.flavour }}-gates", "new-gates", False),
            ("new-${{ matrix.flavour }}gates", "new-gates", True),
            ("}}${{ inputs.context", "protected", True),
        ):
            with self.subTest(pattern=pattern):
                self.assertEqual(checker.possible_context({pattern}, context), expected)

    def test_tab_comments_and_blank_lines_do_not_end_the_jobs_block(self):
        for separator in ("\t# separator", "\t", " \t# separator"):
            with self.subTest(separator=separator):
                self.assertEqual(self.names("  replacement:\n" + separator + "\n    name: protected\n  critical:\n    runs-on: linux\n"), {"replacement": {"protected"}, "critical": {"critical"}})

    def test_implicit_matrix_names_keep_possible_suffixes(self):
        names = self.names("  build:\n    strategy:\n      matrix:\n        os: [linux]\n")["build"]
        self.assertTrue(checker.possible_context(names, "build (linux)"))
        self.assertFalse(checker.possible_context(names, "protected"))

    def test_reusable_jobs_keep_possible_called_job_names(self):
        names = self.names("  call:\n    name: component\n    uses: ./.github/workflows/package.yml\n")["call"]
        self.assertTrue(checker.possible_context(names, "component / protected"))
        self.assertFalse(checker.possible_context(names, "component"))
        with self.assertRaisesRegex(SystemExit, "Cannot resolve"):
            self.base({".github/workflows/old.yml": workflow("  call:\n    name: component\n    uses: ./.github/workflows/package.yml\n")}, "  replacement:\n    name: component / protected\n")

    def test_unknown_matching_base_job_fails_even_when_it_is_the_only_candidate(self):
        for name in ("${{ inputs.name }}", "*alias", "|\n      protected", "!tag protected"):
            with self.subTest(name=name), self.assertRaisesRegex(SystemExit, "Cannot resolve"):
                self.base({".github/workflows/old.yml": workflow("  old:\n    name: " + name + "\n")})

    def test_two_matching_jobs_in_one_workflow_are_ambiguous(self):
        with self.assertRaisesRegex(SystemExit, "More than one base job"):
            self.base({".github/workflows/old.yml": workflow("  first:\n    name: protected\n  second:\n    name: protected\n")})

    def test_duplicate_names_and_keys_are_not_treated_as_unique(self):
        for jobs in ("  old:\n    name: protected\n    name: other\n", "  old:\n    name: protected\n  old:\n    name: other\n", "  old: {name: protected}\n"):
            with self.subTest(jobs=jobs), self.assertRaises(SystemExit):
                self.base({".github/workflows/old.yml": workflow(jobs)})

    def test_unrelated_id_collision_does_not_override_reported_name(self):
        correct = workflow("  original:\n    name: protected\n")
        self.assertEqual(self.base({".github/workflows/other.yml": workflow("  replacement:\n    name: other\n"), ".github/workflows/old.yml": correct}), {"original"})

    def test_unknown_head_name_and_unreadable_base_fail(self):
        with self.assertRaisesRegex(SystemExit, "Cannot determine"):
            self.base({}, "  replacement:\n    name: ${{ inputs.name }}\n")
        with self.assertRaisesRegex(SystemExit, "Cannot read base"):
            self.base({".github/workflows/old.yml": "jobs: *alias\n"})

    def test_same_path_and_context_sources_both_contribute_jobs(self):
        destination = workflow("  existing:\n    runs-on: linux\n")
        source = workflow("  original:\n    name: protected\n")
        with patch.object(checker, "git", side_effect=["new.yml", destination, ".github/workflows/old.yml", source]):
            self.assertEqual(checker.base_jobs(".github/workflows/new.yml", "replacement", workflow("  replacement:\n    name: protected\n")), {"existing", "original"})

    def test_event_forms_and_unknown_namespaces(self):
        for events, expected in (("pull_request", {"standalone"}), ("[push, workflow_call]", {"standalone", "callable"}), ("\n  workflow_call:\n    inputs: {}", {"callable"}), ("*alias", None), ("{workflow_call: {}}", None)):
            with self.subTest(events=events):
                self.assertEqual(checker.workflow_modes("name: fixture\non: " + events + "\njobs:\n  gate:\n"), expected)
        with self.assertRaisesRegex(SystemExit, "invocation namespace"):
            self.base({".github/workflows/old.yml": "name: fixture\non: *alias\njobs:\n  old:\n    name: protected\n"})

    def test_results_needs_exemptions_and_live_declarations(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "workflow.yml"
            path.write_text(workflow("  gate:\n    runs-on: linux\n  test:\n    runs-on: linux\n"), encoding="utf-8")
            with patch.object(checker, "workflow_file", return_value=str(path)), patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "RESULTS": '{"test":{"result":"failure"}}'}):
                with self.assertRaisesRegex(SystemExit, "Jobs did not succeed"):
                    checker.main(["workflow", "gate"])
                checker.main(["workflow", "gate", "--no-results"])
                with patch.dict(os.environ, {"RESULTS": "{}"}):
                    with self.assertRaisesRegex(SystemExit, "Add these jobs to needs"):
                        checker.main(["workflow", "gate"])
                    checker.main(["workflow", "gate", "--exempt", "test"])
                path.write_text("# removed-jobs: test\n" + path.read_text(encoding="utf-8"), encoding="utf-8")
                with self.assertRaisesRegex(SystemExit, "still exist"):
                    checker.main(["workflow", "gate", "--no-results"])


class PipelineGatePredecessorTests(unittest.TestCase):
    def run_fixture(self, gate, context, declaration="", destination="renamed.yml", reusable=False, quoted_path=False, tab_comment=False):
        source_name = "ci-package.yml" if reusable else "ci.yml"
        old_gate = "gates" if reusable else OLD_GATE
        base = (ROOT / ".github/workflows" / source_name).read_text(encoding="utf-8")
        base_name = "c\u00ed.yml" if quoted_path else source_name
        if quoted_path:
            base = base.replace(f".github/workflows/{source_name}", f".github/workflows/{base_name}")
        needs = "    needs: [changes, test, lint, dependency-audit, build]\n" if reusable else NEEDS
        aggregate = base.split(f"\n  {old_gate}:\n", 1)[1]
        self.assertIn(needs, aggregate)
        aggregate = aggregate.replace(needs, "", 1).replace(
            f".github/workflows/{base_name} {old_gate}",
            f".github/workflows/{destination} {gate}",
        )
        event = "workflow_call" if reusable else "pull_request"
        head = f"name: ci\n\non: {event}\n\njobs:\n  {gate}:\n    name: {context}\n" + aggregate
        if tab_comment:
            head = head.replace(f"  {gate}:\n", f"  {gate}:\n\t# separator\n", 1)
        occupied = destination == "devcontainer.yml"
        if occupied:
            head = head.replace("jobs:\n", "jobs:\n  setup:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n", 1)
            head = head.replace(f"    name: {context}\n", f"    name: {context}\n    needs: [setup]\n", 1)
        unchanged = destination == source_name and gate == old_gate and context == old_gate and not declaration
        results = {"setup": {"result": "success"}} if occupied else {}
        if unchanged:
            head = base
            results = {name.strip(): {"result": "success"} for name in needs.split("[", 1)[1].split("]", 1)[0].split(",")}
        if declaration:
            head = f"# removed-jobs: {declaration}\n" + head
        git_command = [rtk, "proxy", "git"] if (rtk := shutil.which("rtk")) else ["git"]
        with tempfile.TemporaryDirectory(prefix="gate-proof-") as directory:
            outer = Path(directory).resolve()
            repo, home = outer / "repo", outer / "home"
            repo.mkdir()
            home.mkdir()
            env = {
                key: os.environ[key]
                for key in ("PATH", "SystemRoot", "TEMP", "TMP")
                if key in os.environ
            }
            env.update(
                HOME=str(home), USERPROFILE=str(home), GIT_CONFIG_NOSYSTEM="1",
                GIT_CONFIG_GLOBAL=os.devnull, GITHUB_EVENT_NAME="pull_request",
                RESULTS=json.dumps(results),
            )

            def git(*args):
                return subprocess.run(
                    [*git_command, *args], cwd=repo, env=env,
                    capture_output=True, text=True, encoding="utf-8", check=True, timeout=10,
                )

            workflows = repo / ".github/workflows"
            workflows.mkdir(parents=True)
            source_workflows = ROOT / ".github/workflows"
            for source in sorted([*source_workflows.glob("*.yml"), *source_workflows.glob("*.yaml")]):
                (workflows / source.name).write_bytes(source.read_bytes())
            if quoted_path:
                (workflows / source_name).unlink()
                (workflows / base_name).write_text(base, encoding="utf-8")
            git("init", "--quiet", "--initial-branch=main")
            git("config", "core.quotePath", "true")
            git("add", "--", ".github/workflows")
            identity = (
                "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "-c", "commit.gpgsign=false",
            )
            git(*identity, "commit", "--quiet", "-m", "Base fixture")
            git("checkout", "--quiet", "-b", "change")
            (workflows / base_name).unlink()
            if unchanged:
                (workflows / destination).write_bytes((ROOT / ".github/workflows" / source_name).read_bytes())
            else:
                (workflows / destination).write_text(head, encoding="utf-8")
            git("add", "--all", "--", ".github/workflows")
            git(*identity, "commit", "--quiet", "--allow-empty", "-m", "Rename fixture")
            git("checkout", "--quiet", "main")
            (repo / ".github/first-parent-control.txt").write_text("base advances\n", encoding="utf-8")
            git("add", "--", ".github/first-parent-control.txt")
            git(*identity, "commit", "--quiet", "-m", "Advance base fixture")
            git(*identity, "merge", "--quiet", "--no-ff", "change", "-m", "Synthetic pull request merge")
            self.assertEqual(len(git("rev-list", "--parents", "-n", "1", "HEAD").stdout.split()), 3)
            self.assertEqual(git("show", f"HEAD^2:.github/workflows/{destination}").stdout, head)
            self.assertEqual(git("show", f"HEAD^1:.github/workflows/{base_name}").stdout, base)
            changes = git("diff", "--no-renames", "--name-status", "-z", "HEAD^1", "HEAD").stdout.split("\0")[:-1]
            self.assertEqual(
                set(zip(changes[::2], changes[1::2])),
                set() if unchanged else {("D", f".github/workflows/{base_name}"), ("M" if occupied else "A", f".github/workflows/{destination}")},
            )
            if quoted_path:
                self.assertIn('".github/workflows/c\\303\\255.yml"', git("ls-tree", "-r", "--name-only", "HEAD^1", "--", ".github/workflows/").stdout)
            return subprocess.run(
                [sys.executable, "-I", str(ROOT / ".github/ci/check_gates.py"),
                 f".github/workflows/{destination}", gate, "--no-results", "--exempt", "codecov"],
                cwd=repo, env=env, capture_output=True, text=True, timeout=10,
            )

    def test_original_gate_id_rejects_undeclared_removals(self):
        result = self.run_fixture(OLD_GATE, OLD_GATE)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)

    def test_renamed_gate_id_keeps_the_protected_context_and_rejects_removals(self):
        result = self.run_fixture("replacement", OLD_GATE)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)

    def test_unrelated_new_context_has_no_predecessor(self):
        result = self.run_fixture("replacement", "new-gates")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_declared_removals_allow_the_renamed_context(self):
        result = self.run_fixture(
            "replacement", OLD_GATE,
            "component, test, dependency-audit, package, lint, windows, rust, windows-workspace, codecov, pipeline-gates",
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_existing_destination_preserves_context_source_removals(self):
        result = self.run_fixture("replacement", OLD_GATE, destination="devcontainer.yml")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("component", result.stderr)

    def test_existing_destination_accepts_declared_source_removals(self):
        result = self.run_fixture("replacement", OLD_GATE,
            "component, test, dependency-audit, package, lint, windows, rust, windows-workspace, codecov, pipeline-gates",
            destination="devcontainer.yml")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_new_standalone_context_does_not_inherit_callee_jobs(self):
        result = self.run_fixture("replacement", "gates")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_renamed_callable_retains_local_lineage(self):
        result = self.run_fixture("replacement", "gates", reusable=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)

    def test_renamed_callable_accepts_declared_removals(self):
        result = self.run_fixture("replacement", "gates", "changes, test, lint, dependency-audit, build, codecov, gates", reusable=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_unchanged_same_path_gate_is_accepted(self):
        result = self.run_fixture(OLD_GATE, OLD_GATE, destination="ci.yml")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_tab_comment_retains_the_protected_context(self):
        result = self.run_fixture("replacement", OLD_GATE, tab_comment=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)

    def test_git_quoted_workflow_retains_predecessor_removals(self):
        result = self.run_fixture("replacement", OLD_GATE, quoted_path=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)

    def test_git_quoted_workflow_accepts_declared_removals(self):
        result = self.run_fixture("replacement", OLD_GATE,
            "component, test, dependency-audit, package, lint, windows, rust, windows-workspace, codecov, pipeline-gates", quoted_path=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
