"""Native guard regression using fabricated CSVs and a disposable script copy."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == "nt", "Windows PowerShell required")
class FinancialFixtureGuardTests(unittest.TestCase):
    def test_unknown_account_is_refused_before_model_connection(self):
        with tempfile.TemporaryDirectory(prefix="powerbi-guard-") as directory:
            target = Path(directory).resolve()
            self.assertEqual(target.parent, Path(tempfile.gettempdir()).resolve())
            (target / "tools").mkdir()
            (target / "samples").mkdir()
            script = target / "tools/test_financial_filters.ps1"
            shutil.copy2(ROOT / "tools/test_financial_filters.ps1", script)
            (target / "samples/sample-chart-of-accounts.csv").write_text(
                "AccountCode,Class,SubClass\n100,Asset,Current Assets\n"
            )
            (target / "samples/sample-general-ledger.csv").write_text(
                "EntityID,PostingDate,AccountCode,Debit,Credit\n"
                "FAB001,2026-07-01,999,1.00,0.00\n"
            )
            result = subprocess.run(
                ["powershell", "-NoProfile", "-File", str(script),
                 "-Server", "localhost:1"],
                capture_output=True, text=True, timeout=30,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("is missing from the chart of accounts", result.stderr)
