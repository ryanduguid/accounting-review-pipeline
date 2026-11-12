# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The retained benchmark measures the frozen Python implementation only."""
import json
import runpy
import subprocess
import sys
import tempfile
import tracemalloc
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "benchmark_aged_review.py"


class PythonReferenceBenchmarkTests(unittest.TestCase):
    def test_a_failed_fabricated_review_cannot_be_reported_as_a_measurement(self):
        with patch.object(sys, "path", sys.path.copy()):
            module = runpy.run_path(str(SCRIPT))
        worker = module["measure"]
        inspect = module["inspect_export"]

        def failed_review(*arguments):
            result = inspect(*arguments)
            result["summary_status"] = "FAIL"
            return result

        content, manifest, control = module["workload"](1)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "source.csv").write_bytes(content)
            for name, value in (("manifest", manifest), ("control", control)):
                (root / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
            with patch.dict(worker.__globals__, {"inspect_export": failed_review}):
                with self.assertRaisesRegex(RuntimeError, "failed its review contract"):
                    worker(root)
        self.assertFalse(tracemalloc.is_tracing())

    def run_benchmark(self, output):
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--output", str(output)],
            capture_output=True, check=False, timeout=60,
        )

    def test_repository_and_existing_outputs_are_refused_without_changes(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as inside:
            output = Path(inside) / "missing-parent" / "measurement.json"
            result = self.run_benchmark(output)
            self.assertEqual(result.returncode, 2)
            self.assertIn(b"outside the checkout", result.stderr)
            self.assertFalse(output.parent.exists())
        with tempfile.TemporaryDirectory() as outside:
            output = Path(outside) / "measurement.json"
            output.write_bytes(b"Previous measurement.\n")
            self.assertEqual(self.run_benchmark(output).returncode, 2)
            self.assertEqual(output.read_bytes(), b"Previous measurement.\n")

    def test_schema_one_keeps_python_measurement_fields_and_workloads(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "new-parent" / "measurement.json"
            result = self.run_benchmark(output)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(b"WORKFLOW MEASUREMENTS QUALIFIED", result.stdout)
            measurement = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(measurement["measurement_schema_version"], 1)
            self.assertIn("python", measurement)
            self.assertIn("Traced Python allocation is not RSS", measurement["boundary"])
            self.assertEqual(len(measurement["workloads"]), 3)
            for workload, count in zip(measurement["workloads"], (10, 1000, 10000), strict=True):
                self.assertEqual(workload["repetitions"], 3)
                self.assertEqual(len(workload["warm_runs"]), 3)
                for run in [*workload["warm_runs"], workload["fresh_process"]]:
                    self.assertEqual(run["rows"], count)
                    self.assertGreater(run["peak_traced_python_bytes"], 0)
                    self.assertEqual(run["workflow_ns"], run["load_ns"] + run["review_ns"] + run["render_write_ns"])


if __name__ == "__main__":
    unittest.main()
