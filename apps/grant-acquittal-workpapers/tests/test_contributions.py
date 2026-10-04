import csv
import io
import shutil
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

from grant_workpaper import main, review

EXAMPLE = Path(__file__).resolve().parents[1] / "examples/two-grants"
HEADER = "contribution_id,grant_id,date,kind,amount,evidence\n"


class ContributionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "inputs"
        shutil.copytree(EXAMPLE, self.source)

    def contributions(self, body):
        (self.source / "contributions.csv").write_text(HEADER + body, encoding="utf-8")

    def test_without_the_file_every_grant_reports_nil(self):
        result = review(self.source)
        self.assertEqual(result["contributions"], [
            {"grant_id": "G-A", "interest": "0", "cash_contribution": "0", "in_kind": "0"},
            {"grant_id": "G-B", "interest": "0", "cash_contribution": "0", "in_kind": "0"},
        ])
        self.assertNotIn("contributions.csv", result["source_sha256"])

    def test_contributions_are_reported_and_leave_cash_and_unspent_funding_alone(self):
        before = review(self.source)["funding_movements"]
        self.contributions(
            "C1,G-A,2026-09-30,interest,41.25,Fabricated bank statement interest\n"
            "C2,G-A,2027-03-31,interest,38.75,Fabricated bank statement interest\n"
            "C3,G-B,2026-11-01,cash_contribution,2500,Fabricated council co-contribution\n"
            "C4,G-B,2026-12-10,in_kind,1200,\n"
        )
        result = review(self.source)
        self.assertEqual(result["contributions"], [
            {"grant_id": "G-A", "interest": "80.00", "cash_contribution": "0", "in_kind": "0"},
            {"grant_id": "G-B", "interest": "0", "cash_contribution": "2500", "in_kind": "1200"},
        ])
        self.assertEqual(result["funding_movements"], before)
        self.assertIn({"code": "MISSING_CONTRIBUTION_EVIDENCE", "reference": "C4"}, result["findings"])
        self.assertIn("contributions.csv", result["source_sha256"])

    def test_invalid_contributions_are_refused(self):
        for body, message in (
            ("C1,G-X,2026-09-30,interest,10,E\n", "declared grant"),
            ("C1,G-A,2025-09-30,interest,10,E\n", "ledger period"),
            ("C1,G-A,2026-09-30,donation,10,E\n", "kind must be one of"),
            ("C1,G-A,2026-09-30,interest,0,E\n", "positive"),
            ("C1,G-A,2026-09-30,interest,10,E\nC1,G-A,2026-10-30,interest,5,E\n", "duplicate"),
        ):
            with self.subTest(body=body):
                self.contributions(body)
                with self.assertRaisesRegex(ValueError, message):
                    review(self.source)

    def assert_input_failure(self, error):
        with self.subTest(surface="API"):
            with self.assertRaises(error):
                review(self.source)
        with self.subTest(surface="CLI"):
            output = self.root / "out"
            with redirect_stderr(io.StringIO()) as diagnostic:
                status = main(["--input", str(self.source), "--output", str(output)])
            self.assertEqual(status, 1)
            self.assertIn("grant-workpaper:", diagnostic.getvalue())
            self.assertFalse(output.exists())

    def link_contributions(self, target):
        try:
            (self.source / "contributions.csv").symlink_to(target)
        except (OSError, NotImplementedError) as exc:
            self.skipTest(f"This host does not permit file symlinks: {exc}")

    def test_dangling_contribution_link_is_refused(self):
        self.link_contributions(self.source / "missing.csv")
        self.assert_input_failure(FileNotFoundError)

    def test_readable_contribution_link_is_included(self):
        target = self.root / "supplement.csv"
        target.write_text(HEADER + "C1,G-A,2026-09-30,interest,12.50,E\n", encoding="utf-8")
        self.link_contributions(target)
        result = review(self.source)
        self.assertEqual(result["contributions"][0]["interest"], "12.50")
        self.assertIn("contributions.csv", result["source_sha256"])

    def test_contribution_inspection_error_is_not_optional_absence(self):
        self.contributions("C1,G-A,2026-09-30,interest,12.50,E\n")
        original = Path.lstat

        def inspect(path, *args, **kwargs):
            if path == self.source / "contributions.csv":
                raise PermissionError("fabricated inspection failure")
            return original(path, *args, **kwargs)

        with patch.object(Path, "lstat", inspect):
            self.assert_input_failure(PermissionError)

    def test_present_contribution_read_errors_are_refused(self):
        self.contributions("C1,G-A,2026-09-30,interest,12.50,E\n")
        original = Path.read_bytes
        for error in (PermissionError, FileNotFoundError):
            with self.subTest(error=error):
                def read(path):
                    if path == self.source / "contributions.csv":
                        raise error("fabricated read failure")
                    return original(path)

                with patch.object(Path, "read_bytes", read):
                    self.assert_input_failure(error)

    def test_the_csv_output_lists_every_grant(self):
        self.contributions("C1,G-A,2026-09-30,interest,12.50,Fabricated interest\n")
        output = self.root / "out"
        self.assertEqual(main(["--input", str(self.source), "--output", str(output)]), 2)
        with (output / "contributions.csv").open(encoding="utf-8", newline="") as stream:
            rows = list(csv.DictReader(stream))
        self.assertEqual([(row["grant_id"], row["interest"]) for row in rows], [("G-A", "12.50"), ("G-B", "0")])
        self.assertIn("contributions.csv reports interest", (output / "workpaper.md").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
