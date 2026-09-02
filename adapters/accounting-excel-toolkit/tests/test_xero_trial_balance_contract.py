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
    """The adapter's Power Query parsers load the exporter's canonical CSV shape.

    These checks read the root data-only contract with the standard library
    only: the exact ten-column header, the pinned bytes, text account codes
    with a leading zero, Tenant plus AccountID as the row identity, and the
    independent movement and YTD balance pairs.
    """

    def test_data_only_contract_is_excel_importable_and_keeps_both_balance_pairs(self):
        expected = {
            "passing.csv": ("2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09", Decimal("0"), Decimal("0")),
            "failing_movement.csv": ("702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4", Decimal("0.01"), Decimal("0")),
            "failing_ytd.csv": ("ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b", Decimal("0"), Decimal("0.01")),
        }
        for name, (digest, movement, ytd) in expected.items():
            with self.subTest(name=name):
                path = CONTRACT / "fixtures" / name
                content = path.read_bytes()
                self.assertNotIn(b"\r\n", content)
                self.assertEqual(hashlib.sha256(content).hexdigest(), digest)
                with path.open(encoding="utf-8-sig", newline="") as source:
                    reader = csv.DictReader(source)
                    self.assertEqual(reader.fieldnames, HEADER)
                    rows = list(reader)
                self.assertEqual(len({(row["Tenant"], row["AccountID"]) for row in rows}), len(rows))
                self.assertIn("090", {row["AccountCode"] for row in rows})
                self.assertEqual(sum((Decimal(row["Debit"]) - Decimal(row["Credit"]) for row in rows), Decimal("0")), movement)
                self.assertEqual(sum((Decimal(row["YTDDebit"]) - Decimal(row["YTDCredit"]) for row in rows), Decimal("0")), ytd)

    def test_schema_file_is_exactly_the_canonical_header(self):
        self.assertEqual((CONTRACT / "schema.csv").read_bytes(), (",".join(HEADER) + "\n").encode("ascii"))


if __name__ == "__main__":
    unittest.main()
