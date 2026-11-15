# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

import base64
import io
import json
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

import differential


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


if __name__ == "__main__":
    unittest.main()
