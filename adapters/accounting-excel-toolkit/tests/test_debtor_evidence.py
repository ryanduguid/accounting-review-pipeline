import json
import subprocess  # nosec B404 - fixed local CLI regression commands.
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import debtor_evidence  # noqa: E402 - standalone tools are imported after their path bootstrap
import replay_aged_review  # noqa: E402
import xero_aged_receivables  # noqa: E402


class DebtorEvidenceTests(unittest.TestCase):
    def run_output_cli(self, tool, output):
        arguments = [sys.executable, str(ROOT / "tools" / f"{tool}.py")]
        if tool == "debtor_evidence":
            arguments += ["draft", str(ROOT / "samples/sample-aged-receivables-review.csv")]
        elif tool == "replay_aged_review":
            arguments += ["--case", "clean"]
        arguments += ["--output", str(output)]
        return subprocess.run(arguments, capture_output=True, text=True, timeout=60, check=False)  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit

    def test_all_output_clis_reject_repository_paths_before_creating_parents(self):
        repository = ROOT.parents[1]
        with tempfile.TemporaryDirectory(prefix="debtor-output-test-", dir=repository) as temporary:
            for tool in ("debtor_evidence", "benchmark_aged_review", "replay_aged_review"):
                with self.subTest(tool=tool):
                    output = Path(temporary) / tool / "new-output"
                    result = self.run_output_cli(tool, output)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("outside the checkout", result.stdout + result.stderr)
                    self.assertFalse(output.parent.exists())

    def test_all_output_clis_accept_new_external_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            for tool in ("debtor_evidence", "benchmark_aged_review", "replay_aged_review"):
                with self.subTest(tool=tool):
                    output = Path(temporary) / tool / "new-output"
                    result = self.run_output_cli(tool, output)
                    self.assertEqual(result.returncode, 2 if tool == "debtor_evidence" else 0,
                                     result.stdout + result.stderr)
                    self.assertTrue(output.exists())

    def test_all_output_clis_preserve_existing_external_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "existing-output"
            output.write_bytes(b"Existing operator evidence.\n")
            for tool in ("debtor_evidence", "benchmark_aged_review", "replay_aged_review"):
                with self.subTest(tool=tool):
                    result = self.run_output_cli(tool, output)
                    self.assertEqual(result.returncode, 1 if tool == "debtor_evidence" else 2)
                    self.assertEqual(output.read_bytes(), b"Existing operator evidence.\n")

    def test_failure_catalogue_runs_actual_cli_and_expected_exits(self):
        outcomes = replay_aged_review.replay()["cases"]
        self.assertEqual(len(outcomes), 12)
        self.assertEqual({item["actual_exit"] for item in outcomes}, {0, 1, 2})
        self.assertTrue(all(item["expected_exit"] == item["actual_exit"] for item in outcomes))
        with self.assertRaises(ValueError):
            replay_aged_review.replay("unknown")

    def setUp(self):
        self.content = (ROOT / "samples/sample-aged-receivables-review.csv").read_bytes()
        self.manifest = json.loads((ROOT / "samples/sample-aged-receivables-manifest.json").read_text())
        self.control = json.loads((ROOT / "samples/sample-aged-receivables-control.json").read_text())

    def test_source_draft_cannot_confirm_independent_evidence(self):
        draft = debtor_evidence.draft_manifest(self.content)
        self.assertEqual(draft["entity"], self.manifest["entity"])
        self.assertEqual(draft["as_at"], "2026-09-30")
        for key in ("generated_at", "accounting_basis", "currency", "filters", "population", "on_screen_total"):
            self.assertEqual(draft[key], "")
        self.assertIsNone(draft["footer_record"])
        self.assertIsNone(draft["on_screen_rows"])
        result = xero_aged_receivables.inspect_export(self.content, draft)
        self.assertEqual((result["summary_status"], result["debtor_decisions"]), ("REVIEW", "REVIEW"))
        self.assertTrue(result["exceptions"])

    def test_requests_preserve_duplicate_rows_roles_and_untrusted_text(self):
        result = xero_aged_receivables.inspect_export(self.content, self.manifest, self.control)
        requests = debtor_evidence.evidence_requests(result)
        self.assertEqual([request["source_records"] for request in requests], [[7], [8], [9]])
        self.assertEqual([request["contact"] for request in requests], ["00123", "00123", "Total"])
        result["rows"][0]["Contact"] = "=bad|\n\x1b[31m"
        text = debtor_evidence.briefing(result)
        self.assertIn("\\n\\u001b[31m", text)
        self.assertNotIn("\x1b", text)
        self.assertEqual(result["debtor_decisions"], "REVIEW")

    def test_new_external_output_and_existing_output_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "draft"
            args = ["draft", str(ROOT / "samples/sample-aged-receivables-review.csv"), "--output", str(output)]
            self.assertEqual(debtor_evidence.main(args), 2)
            self.assertTrue((output / "manifest-draft.json").is_file())
            self.assertEqual(debtor_evidence.main(args), 1)
            self.assertEqual(debtor_evidence.main([*args[:-1], str(ROOT / "blocked-output")]), 1)
