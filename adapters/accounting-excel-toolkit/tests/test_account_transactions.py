import csv
import hashlib
import importlib.util
import io
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("xero_transactions", ROOT / "tools/xero_account_transactions.py")
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AccountTransactionsTests(unittest.TestCase):
    def setUp(self):
        self.content = (ROOT / "samples/xero-account-transactions.csv").read_bytes()

    def mapping(self, content=None):
        return {"source_sha256": hashlib.sha256(content or self.content).hexdigest(),
                "lines": {"9": "source-line-1", "10": "source-line-2"}}

    def test_inspection_and_conversion(self):
        result = module.inspect_export(self.content, tenant="Fabricated Services", account_name="Clearing")
        self.assertEqual(result["debit"], "110.00")
        rows = module.convert(self.content, self.mapping(), tenant="Fabricated Services",
                              account_name="Clearing", account_id="090", currency="AUD")
        self.assertEqual(rows[0]["AccountID"], "090")
        self.assertEqual(rows[1]["Credit"], "100.00")
        self.assertEqual(rows[0]["Date"], "2026-09-01")

    def test_extra_title_row_preserves_inspection(self):
        result = module.inspect_export(b"Prepared copy,,,,,,,,\n" + self.content,
                                       tenant="Fabricated Services", account_name="Clearing")
        self.assertEqual(result["detail"][0]["source_row"], 10)

    def test_native_excel_display_dates_and_grouped_amounts(self):
        content = self.content.replace(b"01/09/2026", b"1 Sep 2026").replace(b"02/09/2026", b"2 Sep 2026")
        result = module.inspect_export(content, tenant="Fabricated Services", account_name="Clearing")
        self.assertEqual(result["detail"][0]["Date"], "2026-09-01")
        self.assertEqual(str(module.amount("6,661.60")), "6661.60")
        with self.assertRaises(ValueError):
            module.amount("66,61.60")

    def test_missing_field_and_changed_header_fail(self):
        for old, new in [(b"110.00,0,110.00", b"110.00,110.00"), (b"Running Balance", b"Balance")]:
            with self.subTest(old=old), self.assertRaises(ValueError):
                module.inspect_export(self.content.replace(old, new), tenant="Fabricated Services", account_name="Clearing")

    def test_subtotal_and_period_fail(self):
        for old, new in [(b"Total Clearing,,,,110.00", b"Total Clearing,,,,0"), (b"01/09/2026", b"01/10/2026")]:
            with self.subTest(old=old), self.assertRaises(ValueError):
                module.inspect_export(self.content.replace(old, new), tenant="Fabricated Services", account_name="Clearing")

    def test_mapping_identity_coverage_and_duplicates_fail(self):
        changes = [{"source_sha256": "wrong"}, {"lines": {"9": "id"}}, {"lines": {"9": "id", "10": "id"}}]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                module.convert(self.content, {**self.mapping(), **change}, tenant="Fabricated Services",
                               account_name="Clearing", account_id="090", currency="AUD")

    def test_row_reordering_invalidates_mapping(self):
        rows = list(csv.reader(io.StringIO(self.content.decode())))
        rows[8], rows[9] = rows[9], rows[8]
        output = io.StringIO()
        csv.writer(output).writerows(rows)
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            module.convert(output.getvalue().encode(), self.mapping(), tenant="Fabricated Services",
                           account_name="Clearing", account_id="090", currency="AUD")


if __name__ == "__main__":
    unittest.main()
