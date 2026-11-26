# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.

import base64
import importlib.util
import io
import json
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "grant_timeout_differential", Path(__file__).with_name("differential.py")
)
assert SPEC is not None and SPEC.loader is not None
differential = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(differential)


class TimeoutArtifacts(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="grant-timeout-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifact = self.root / "timeout.json"
        self.case = differential.Case()
        self.case.label = "7-output"
        self.case.files["ledger.csv"] = b"\x00\xff,\r\n"
        self.case.extra = {"out/": None, "blocker": b"retain\x00"}
        self.case.input_name = b"in-\xff"
        self.case.argv = ["--input", self.case.input_name, "--output", "out"]
        self.script, self.binary = Path("frozen.py"), Path("grant-workpaper")

    def run_pair(self):
        return differential.run_pair(self.script, self.binary, self.case, self.root / "py", self.root / "rs",
                                     seed=20261012, index=7, timeout_artifact=self.artifact)

    def test_native_timeout_retains_exact_case_and_available_output(self):
        oracle = subprocess.CompletedProcess([], 0, b"oracle", b"")
        error = subprocess.TimeoutExpired([str(self.binary), *self.case.argv], 120,
                                          output=b"partial\xff", stderr=b"issue\x00")
        with patch.object(differential.subprocess, "run", side_effect=[oracle, error]) as run:
            with redirect_stderr(io.StringIO()), self.assertRaises(subprocess.TimeoutExpired) as raised:
                self.run_pair()
        self.assertIs(raised.exception, error)
        report = json.loads(self.artifact.read_text(encoding="utf-8"))
        self.assertEqual((report["seed"], report["index"], report["label"], report["phase"]),
                         (20261012, 7, "7-output", "rust"))
        self.assertEqual(report["timeout_seconds"], 120)
        self.assertEqual(base64.b64decode(report["files"]["ledger.csv"]), b"\x00\xff,\r\n")
        self.assertIsNone(report["extra"]["out/"])
        self.assertEqual(base64.b64decode(report["extra"]["blocker"]), b"retain\x00")
        self.assertEqual(report["argv"][0], {"text": "--input"})
        self.assertEqual(base64.b64decode(report["argv"][1]["bytes_base64"]), b"in-\xff")
        self.assertEqual(report["input_name"], report["argv"][1])
        self.assertEqual(base64.b64decode(report["stdout_base64"]), error.stdout)
        self.assertEqual(base64.b64decode(report["stderr_base64"]), error.stderr)
        self.assertEqual(base64.b64decode(report["completed"][0]["stdout_base64"]), b"oracle")
        self.assertTrue(all(call.kwargs["timeout"] == 120 for call in run.call_args_list))

    def test_python_timeout_records_unavailable_output_and_skips_native(self):
        error = subprocess.TimeoutExpired(["python"], 120)
        with patch.object(differential.subprocess, "run", side_effect=error) as run:
            with redirect_stderr(io.StringIO()), self.assertRaises(subprocess.TimeoutExpired):
                self.run_pair()
        report = json.loads(self.artifact.read_text(encoding="utf-8"))
        self.assertEqual(report["phase"], "python")
        self.assertEqual(report["completed"], [])
        self.assertIsNone(report["stdout_base64"])
        self.assertIsNone(report["stderr_base64"])
        self.assertEqual(run.call_count, 1)

    def test_existing_artifact_is_preserved_and_original_timeout_propagates(self):
        self.artifact.write_bytes(b"keep")
        error = subprocess.TimeoutExpired(["python"], 120)
        with patch.object(differential.subprocess, "run", side_effect=error):
            with redirect_stderr(io.StringIO()) as stderr, self.assertRaises(subprocess.TimeoutExpired) as raised:
                self.run_pair()
        self.assertIs(raised.exception, error)
        self.assertEqual(self.artifact.read_bytes(), b"keep")
        self.assertIn("could not retain timeout artifact", stderr.getvalue())

    def test_success_returns_both_results_without_an_artifact(self):
        oracle = subprocess.CompletedProcess([], 0, b"oracle", b"")
        native = subprocess.CompletedProcess([], 0, b"native", b"")
        with patch.object(differential.subprocess, "run", side_effect=[oracle, native]):
            results = self.run_pair()
        self.assertEqual(results, [oracle, native])
        self.assertFalse(self.artifact.exists())


class ReferenceCells(unittest.TestCase):
    """The comparator removes only the Rust command's reference cells."""

    def test_reference_columns_are_removed_and_every_other_byte_kept(self):
        rust = differential.csv_text([["grant_id", "allocated", "allocated_ledger_rows"], ["G-A", "600", "['L1']"]])
        frozen = differential.csv_text([["grant_id", "allocated"], ["G-A", "600"]])
        self.assertEqual(differential.without_references("out/budget_lines.csv", rust), frozen)
        document = {"ledger_total": "6500", "ledger_total_ledger_rows": ["L1"], "budget_lines": [{"a": "1", "a_ledger_rows": []}]}
        expected = differential.json_text({"ledger_total": "6500", "budget_lines": [{"a": "1"}]})
        self.assertEqual(differential.without_references("out/workpaper.json", differential.json_text(document)), expected)

    def test_a_file_the_frozen_writer_would_not_produce_is_compared_as_it_is(self):
        rows = [["grant_id", "allocated_ledger_rows"], ["G-A", "[]"]]
        lf = differential.csv_text(rows).replace(b"\r\n", b"\n")
        self.assertEqual(differential.without_references("out/budget_lines.csv", lf), lf)
        quoted = b'"grant_id",allocated_ledger_rows\r\nG-A,[]\r\n'
        self.assertEqual(differential.without_references("out/budget_lines.csv", quoted), quoted)
        compact = json.dumps({"ledger_total": "1", "ledger_total_ledger_rows": []}).encode() + b"\n"
        self.assertEqual(differential.without_references("out/workpaper.json", compact), compact)
        self.assertEqual(differential.without_references("out/budget_lines.csv", b"\xff"), b"\xff")

    def test_other_files_are_untouched(self):
        data = b"code,reference\r\nX,a_ledger_rows\r\n"
        self.assertEqual(differential.without_references("out/findings.csv", data), data)


class UsageIndent(unittest.TestCase):
    """Wrapped usage lines differ only by the program name's width."""

    def test_only_the_usage_continuation_is_reindented(self):
        def usage(prog):
            pad = " " * len(f"usage: {prog} ")
            return (f"usage: {prog} [-h] --input INPUT\n{pad}[--ignore-file IGNORE_FILE]\n\n"
                    "options:\n  --ignore-file IGNORE_FILE\n                        help text\n").encode()

        python = differential.normalise(usage("grant_workpaper.py"), Path("/no-such-cwd"), ("grant_workpaper.py",))
        rust = differential.normalise(usage("grant-workpaper"), Path("/no-such-cwd"), ("grant-workpaper",))
        self.assertEqual(python, rust)
        self.assertIn(b"\n                        help text\n", rust)

    def test_a_misaligned_continuation_still_differs(self):
        # 11 spaces is not a.py's indent of 12, so only the Rust line is re-indented.
        python = differential.normalise(b"usage: a.py [-h]\n           [--x X]\n", Path("/no-such-cwd"), ("a.py",))
        rust = differential.normalise(b"usage: ab [-h]\n          [--x X]\n", Path("/no-such-cwd"), ("ab",))
        self.assertNotEqual(python, rust)


if __name__ == "__main__":
    unittest.main()
