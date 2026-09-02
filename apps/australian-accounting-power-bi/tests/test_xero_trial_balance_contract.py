import csv
import hashlib
import unittest
from decimal import Decimal
from pathlib import Path


CONTRACT = Path(__file__).resolve().parents[3] / "contracts" / "xero-trial-balance-v1"
HEADER = [
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
]


class XeroTrialBalanceContractTests(unittest.TestCase):
    """The reference application imports the exporter's canonical CSV shape.

    These checks read the root data-only contract with the standard library
    only: the pinned bytes, the exact ten-column header, one report date and
    one tenant per file, a stable non-empty AccountID on every row, a text
    AccountCode that keeps its leading zero, and decimal balance columns.
    """

    def test_model_import_boundary_uses_the_pinned_data_only_shape(self):
        expected = {
            "passing.csv": "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
            "failing_movement.csv": "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
            "failing_ytd.csv": "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
        }
        for name, digest in expected.items():
            with self.subTest(name=name):
                fixture = CONTRACT / "fixtures" / name
                self.assertEqual(hashlib.sha256(fixture.read_bytes()).hexdigest(), digest)
                with fixture.open(encoding="utf-8-sig", newline="") as source:
                    reader = csv.DictReader(source)
                    self.assertEqual(reader.fieldnames, HEADER)
                    rows = list(reader)
                self.assertEqual(len({row["ReportDate"] for row in rows}), 1)
                self.assertEqual(len({row["Tenant"] for row in rows}), 1)
                self.assertTrue(all(row["AccountID"] for row in rows))
                self.assertEqual(len({(row["Tenant"], row["AccountID"]) for row in rows}), len(rows))
                self.assertIn("090", {row["AccountCode"] for row in rows})
                for row in rows:
                    for column in ("Debit", "Credit", "YTDDebit", "YTDCredit"):
                        Decimal(row[column])

    def test_documentation_uses_the_canonical_home_contract_and_active_ci(self):
        component = Path(__file__).resolve().parents[1]
        readme = (component / "README.md").read_text(encoding="utf-8")
        self.assertIn("accounting-review-pipeline/tree/main/apps/australian-accounting-power-bi", readme)
        self.assertIn("../../contracts/xero-trial-balance-v1/", readme)
        self.assertIn("../../.github/workflows/australian-accounting-power-bi.yml", readme)
        self.assertIn("accounting-review-pipeline/tree/main/packages/xero-trial-balance-export", readme)
        self.assertNotIn("australian-accounting-power-bi/actions/workflows/verify.yml", readme)


if __name__ == "__main__":
    unittest.main()
