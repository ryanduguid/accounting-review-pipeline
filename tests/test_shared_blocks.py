"""Behavioural conformance for the independently released review commands."""
from __future__ import annotations

import tempfile
import unittest
import sys
from decimal import Decimal
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "packages/monthly-close-control-plane"))
from closecontrol import loader, report, viewer  # noqa: E402
from tests.native_review_ready import probe


class SharedBlockTests(unittest.TestCase):
    def test_money_grammar_and_exact_values_agree(self) -> None:
        values = (None, "", "0", "-0.00", "1,234.50", "(1,234.50)", "+1.2",
                  "١٢.٥", "1e2", "12,34", "(+1)", "--1", "NaN", "1_000")
        for value in values:
            native = probe({"op": "money", "value": value})
            with self.subTest(value=value):
                try:
                    expected = loader.parse_money(value, field="Amount", row_number=2,
                                                  path=Path("input.csv"))
                except loader.ControlInputError:
                    self.assertIn("error", native)
                else:
                    self.assertEqual(native, {"ok": str(expected)})

    def test_money_report_precision_agrees(self) -> None:
        for value in ("0", "-0", "1.2", "1.2300", "1234567890123456789.0123"):
            with self.subTest(value=value):
                self.assertEqual(probe({"op": "format_money", "value": value}),
                                 {"ok": report._money(Decimal(value))})

    def test_calendar_dates_agree(self) -> None:
        for value in ("2024-02-29", "2026-02-29", "20260923", "2026-W39-3",
                      "2026-09-23", "0000-01-01", "9999-12-31"):
            native = probe({"op": "date", "value": value})
            with self.subTest(value=value):
                try:
                    expected = loader.parse_iso_date(value, field="Date", path=Path("input.csv"),
                                                     row_number=2)
                except loader.ControlInputError:
                    self.assertIn("error", native)
                else:
                    self.assertEqual(native, {"ok": expected.isoformat()})

    def test_csv_headers_width_and_identifiers_agree(self) -> None:
        header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"
        row = "2026-09-23,Fixture,Assets,one,Cash,090,0,0,0,0"
        cases = (header + "\n" + row + "\n", header + ",Extra\n" + row + ",x\n",
                 header + "\n" + row + ",extra\n", header + "\n" + row.rsplit(",", 1)[0],
                 header.replace("Tenant", "AccountID") + "\n" + row,
                 header + "\n" + row.replace("one", "one\u200b"),
                 header + "\n" + row + "\n" + row,
                 "\ufeff" + header + "\r\n" + row + "\r\n")
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.csv"
            for content in cases:
                source.write_bytes(content.encode())
                native = probe({"op": "csv", "kind": "trial_balance", "path": str(source),
                                "hex": source.read_bytes().hex()})
                with self.subTest(content=content):
                    try:
                        rows = loader.load_canonical_tb(source)
                    except loader.ControlInputError:
                        self.assertIn("error", native)
                    else:
                        self.assertEqual([(r.tenant, r.account_id, r.account_code) for r in rows],
                                         [(r["tenant"], r["account_id"], r["account_code"])
                                          for r in native["ok"]])

    def test_both_saved_json_parsers_refuse_ambiguous_or_unreadable_documents(self) -> None:
        for payload in (b'[]', b'null', b'{}', b'{"unexpected":1}', b'{"x":1,"x":2}',
                        b'{"x":{"a":1,"a":2}}', b'\xff', b'{"x":', b'\xef\xbb\xbf{}'):
            with self.subTest(payload=payload):
                with self.assertRaises(loader.ControlInputError):
                    viewer._parse_json(payload)
                self.assertIn("error", probe({"op": "parse_pack_json", "hex": payload.hex()}))

    def test_checkout_guards_agree_for_every_marker_and_ancestor(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            outside = root / "outside"
            self.assertEqual(probe({"op": "output_guard", "path": str(outside)}),
                             {"ok": str(report.require_output_outside_repository(outside))})
            for marker in (".git", ".hg", ".svn", ".bzr"):
                checkout = root / marker[1:]
                checkout.mkdir()
                (checkout / marker).mkdir()
                destination = checkout / "nested" / "output"
                with self.subTest(marker=marker):
                    with self.assertRaises(ValueError):
                        report.require_output_outside_repository(destination)
                    self.assertIn("error", probe({"op": "output_guard", "path": str(destination)}))


if __name__ == "__main__":
    unittest.main()
