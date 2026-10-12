"""export_tb edges the main suite leaves out: report shapes, refusals, quiet detail."""

import os
import tempfile
import unittest
from decimal import Decimal
from unittest import mock

import export_tb

HEADER = ["Account", "Debit", "Credit", "YTD Debit", "YTD Credit"]


def _header(titles):
    return {"RowType": "Header", "Cells": [{"Value": t} for t in titles]}


def _row(values, account_id=None):
    cells = [{"Value": v} for v in values]
    if account_id is not None and cells:
        cells[0]["Attributes"] = [{"Value": account_id, "Id": "account"}]
    return {"RowType": "Row", "Cells": cells}


class FlattenReportEdgeTest(unittest.TestCase):
    def test_rows_that_are_neither_accounts_nor_summaries_are_skipped(self):
        report = {
            "Rows": [
                _header(HEADER),
                {"RowType": "Spacer"},
                {
                    "RowType": "Section",
                    "Title": "Assets",
                    "Rows": [
                        {"RowType": "Header", "Cells": [{"Value": "not an account"}]},
                        _row(["Cash (090)", "10.00", "", "10.00", ""], account_id="guid-090"),
                    ],
                },
            ]
        }
        titles, flat = export_tb.flatten_report(report)
        self.assertEqual(titles, HEADER)
        self.assertEqual(len(flat), 1)
        self.assertEqual(flat[0]["Account"], "Cash (090)")
        self.assertEqual(flat[0]["AccountID"], "guid-090")
        self.assertEqual(flat[0]["Section"], "Assets")

    def test_a_row_with_no_cells_has_no_account_id(self):
        report = {
            "Rows": [
                _header([]),
                {"RowType": "Section", "Title": "Assets", "Rows": [_row([])]},
            ]
        }
        self.assertEqual(
            export_tb.flatten_report(report), ([], [{"Section": "Assets", "AccountID": ""}])
        )


class QuietDiagnosticWinerrorTest(unittest.TestCase):
    def test_a_windows_error_code_is_kept_under_quiet(self):
        error = OSError(13, "Access is denied")
        error.winerror = 5
        with mock.patch.object(export_tb, "QUIET", True):
            self.assertEqual(
                export_tb._diagnostic_detail(error), "winerror 5: errno 13: Access is denied"
            )


class CheckoutProbeFailureTest(unittest.TestCase):
    def test_an_uninspectable_directory_refuses_instead_of_approving(self):
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(
                    export_tb.os, "lstat", side_effect=PermissionError(13, "Access is denied")
                ), \
                self.assertRaises(ValueError) as raised:
            export_tb._enclosing_checkout(directory)
        message = str(raised.exception)
        self.assertTrue(message.startswith(f"{directory} cannot be examined for a .git entry"))
        self.assertIn("Access is denied", message)
        self.assertIn("outside version control", message)
        self.assertIsInstance(raised.exception.__cause__, PermissionError)


class InexactBalanceTest(unittest.TestCase):
    def test_a_difference_beyond_exact_precision_writes_nothing(self):
        huge = Decimal("1E+70")
        with self.assertRaises(SystemExit) as raised:
            export_tb.check_balanced((huge, Decimal(1), Decimal(0), Decimal(0)))
        self.assertEqual(
            raised.exception.code,
            "Nothing written - the movement difference cannot be computed exactly, "
            "so the balance check cannot be trusted.",
        )


class OutputDirectoryCreationTest(unittest.TestCase):
    def test_a_directory_that_cannot_be_created_is_one_line_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as root:
            out_path = os.path.join(root, "exports", "tb.csv")
            with mock.patch.object(
                export_tb.os, "makedirs", side_effect=PermissionError(13, "Access is denied")
            ), mock.patch.object(export_tb.tempfile, "mkstemp") as mkstemp, \
                    self.assertRaises(SystemExit) as raised:
                export_tb.write_csv([], out_path)
            mkstemp.assert_not_called()
            self.assertEqual(os.listdir(root), [])
        message = raised.exception.code
        self.assertTrue(
            message.startswith(
                f"error: cannot create the output directory {os.path.dirname(out_path)} ("
            ),
            message,
        )
        self.assertIn("Access is denied", message)


if __name__ == "__main__":
    unittest.main()
