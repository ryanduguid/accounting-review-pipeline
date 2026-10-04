import csv
import json
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import grant_workpaper
from grant_workpaper import main, review, write

EXAMPLE = Path(__file__).resolve().parents[1] / "examples/two-grants"


class GrantWorkpaperTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "inputs"
        shutil.copytree(EXAMPLE, self.source)

    def edit_csv(self, name, change):
        path = self.source / name
        with path.open(newline="", encoding="utf-8") as stream:
            reader = csv.DictReader(stream)
            fields, records = reader.fieldnames, list(reader)
        change(records)
        with path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fields)
            writer.writeheader()
            writer.writerows(records)

    def test_independent_totals_and_cash_are_distinct(self):
        result = review(self.source)
        self.assertEqual(result["ledger_total"], "6500")
        self.assertEqual(result["allocated_total"], "6300")
        self.assertEqual(result["unallocated_total"], "200")
        a, b = result["funding_movements"]
        self.assertEqual((a["closing_cash_allocation"], a["unspent_funding_workpaper"]), ("14220", "9100"))
        self.assertEqual((b["closing_cash_allocation"], b["unspent_funding_workpaper"]), ("7680", "7600"))
        self.assertEqual({row["code"] for row in result["findings"]}, {
            "MISSING_SOURCE_EVIDENCE", "OUTSIDE_AGREEMENT_PERIOD",
            "MISSING_ALLOCATION_EVIDENCE", "UNAPPROVED_ALLOCATION", "UNALLOCATED_SOURCE"})

    def test_shared_invoice_cannot_be_overallocated(self):
        self.edit_csv("allocations.csv", lambda rows: rows[0].update(amount="601"))
        with self.assertRaisesRegex(ValueError, "exceed"):
            review(self.source)

    def test_cash_allocation_cannot_exceed_paid_source(self):
        self.edit_csv("allocations.csv", lambda rows: rows[0].update(cash_allocated="481"))
        with self.assertRaisesRegex(ValueError, "exceed"):
            review(self.source)

    def test_grant_period_is_separate_from_financial_year(self):
        result = review(self.source)
        b = next(row for row in result["budget_lines"] if row["grant_id"] == "G-B" and row["budget_line"] == "Delivery")
        self.assertEqual(b["allocated"], "2400")
        self.assertEqual(b["within_period"], "400")
        self.assertEqual(result["ledger_total"], "6500")

    def test_unapproved_allocation_does_not_hide_supported_work(self):
        result = review(self.source)
        a = next(row for row in result["budget_lines"] if row["grant_id"] == "G-A" and row["budget_line"] == "Delivery")
        self.assertEqual(a["allocated"], "3600")
        self.assertEqual(a["approved_evidenced_in_period"], "600")

    def test_duplicate_ledger_id_is_refused(self):
        self.edit_csv("ledger.csv", lambda rows: rows.append(dict(rows[0])))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            review(self.source)

    def test_zero_is_valid_but_missing_or_nonfinite_is_not(self):
        self.assertEqual(review(self.source)["status"], "REVIEW")
        for value in ("", "NaN", "Infinity", "-1", "1e3"):
            with self.subTest(value=value):
                self.edit_csv("ledger.csv", lambda rows: rows[0].update(amount=value))
                with self.assertRaises(ValueError):
                    review(self.source)

    def test_duplicate_json_fields_and_unknown_budget_fail(self):
        path = self.source / "agreements.json"
        saved = path.read_bytes()
        path.write_text('{"schema_version":1,"schema_version":1}')
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            review(self.source)
        path.write_bytes(saved)
        self.edit_csv("allocations.csv", lambda rows: rows[0].update(budget_line="Unknown"))
        with self.assertRaises(ValueError):
            review(self.source)

    def test_deeply_nested_agreements_are_malformed_input_not_a_traceback(self):
        # How deep the parser can go depends on the platform's stack, so the
        # decoder's RecursionError is raised directly rather than by a fixture.
        too_deep = RecursionError("maximum recursion depth exceeded while decoding a JSON array")
        with patch("sys.stderr") as stderr, patch.object(grant_workpaper.json, "loads", side_effect=too_deep):
            self.assertEqual(main(["--input", str(self.source), "--output", str(self.root / "out")]), 1)
        self.assertIn("nested too deeply", "".join(call.args[0] for call in stderr.write.call_args_list))

    def test_clean_evidence_can_reconcile_without_approving_acquittal(self):
        self.edit_csv("ledger.csv", lambda rows: rows[2].update(evidence="Fabricated receipt"))
        def fix(rows):
            rows[2]["grant_id"] = "G-A"
            rows[3].update(approved="yes", evidence="Fabricated reviewed allocation")
            rows[4].update(amount="500", cash_allocated="500")
        self.edit_csv("allocations.csv", fix)
        result = review(self.source)
        self.assertEqual(result["status"], "RECONCILED")
        self.assertIn("No grant eligibility", result["scope"])

    def test_csv_text_is_inert_and_previous_run_is_immutable(self):
        self.edit_csv("allocations.csv", lambda rows: rows[0].update(evidence="=1+1"))
        result = review(self.source)
        output = self.root / "output"
        write(result, output)
        self.assertIn("'=1+1", (output / "allocation_evidence.csv").read_text())
        saved = (output / "workpaper.json").read_bytes()
        with self.assertRaises(FileExistsError):
            write(result, output)
        self.assertEqual(saved, (output / "workpaper.json").read_bytes())
        self.assertEqual(json.loads(saved)["allocation_evidence"][0]["evidence"], "=1+1")

    def edit_agreement(self, grant_id, **changes):
        path = self.source / "agreements.json"
        document = json.loads(path.read_text(encoding="utf-8"))
        next(a for a in document["agreements"] if a["grant_id"] == grant_id).update(changes)
        path.write_text(json.dumps(document), encoding="utf-8")

    def codes(self, reference):
        return {row["code"] for row in review(self.source)["findings"] if row["reference"] == reference}

    def test_budget_boundary_is_exact_to_the_cent(self):
        # G-A Delivery carries 3,600 of allocations.
        self.edit_agreement("G-A", budget={"Delivery": "3600", "Administration": "2000"})
        self.assertNotIn("OVER_BUDGET", self.codes("G-A/Delivery"))
        self.edit_agreement("G-A", budget={"Delivery": "3599.99", "Administration": "2000"})
        self.assertIn("OVER_BUDGET", self.codes("G-A/Delivery"))

    def test_both_agreement_dates_are_inside_the_period(self):
        # A2 allocates L1 to G-B, which starts 2026-10-01; A5 allocates L4 to
        # G-A, which ends 2027-06-30.
        def on_boundaries(rows):
            rows[0]["date"] = "2026-10-01"
            rows[3]["date"] = "2027-06-30"
        self.edit_csv("ledger.csv", on_boundaries)
        outside = {row["reference"] for row in review(self.source)["findings"]
                   if row["code"] == "OUTSIDE_AGREEMENT_PERIOD"}
        self.assertEqual(outside, {"A3"})

    def test_cash_and_unspent_shortfalls_are_each_flagged_to_the_cent(self):
        # G-B allocates 2,400 of expenditure and 2,320 of cash.
        cases = {
            "cash only": ("2319.99", {"opening_unspent_funding": "1000"}, "-0.01"),
            "unspent only": ("2399.99", {"opening_cash": "5000"}, None),
        }
        for label, (received, opening, closing_cash) in cases.items():
            with self.subTest(label):
                shutil.rmtree(self.source)
                shutil.copytree(EXAMPLE, self.source)
                self.edit_csv("receipts.csv", lambda rows: rows[1].update(amount=received))
                self.edit_agreement("G-B", **opening)
                result = review(self.source)
                self.assertIn({"code": "FUNDING_SHORTFALL", "reference": "G-B"}, result["findings"])
                if closing_cash is not None:
                    output = self.root / "output-cash"
                    write(result, output)
                    with (output / "funding_movements.csv").open(newline="", encoding="utf-8") as stream:
                        row = next(r for r in csv.DictReader(stream) if r["grant_id"] == "G-B")
                    # A number stays a number, so spreadsheet totals include it.
                    self.assertEqual(row["closing_cash_allocation"], closing_cash)

    def test_formula_like_text_is_escaped_including_at_and_signed_expressions(self):
        for text in ("@SUM(A1)", "-1+1", "=1+1", "+1", "-001", "-01.5"):
            with self.subTest(text=text):
                self.assertEqual(grant_workpaper.safe_cell(text), "'" + text)
        for number in ("-1320", "0", "12.50"):
            with self.subTest(number=number):
                self.assertEqual(grant_workpaper.safe_cell(number), number)

    def test_cli_codes_and_no_output_for_invalid_sources(self):
        self.assertEqual(main(["--input", str(self.source), "--output", str(self.root / "valid")]), 2)
        self.edit_csv("allocations.csv", lambda rows: rows[0].update(amount="9999"))
        self.assertEqual(main(["--input", str(self.source), "--output", str(self.root / "invalid")]), 1)
        self.assertFalse((self.root / "invalid").exists())


    def test_json_identity_and_evidence_require_plain_nonblank_text(self):
        path = self.source / "agreements.json"
        original = path.read_text()
        for field in ("entity", "grant_id", "agreement_evidence"):
            for value in ("   ", 123, "Valid\n## Fake report", "\u200b", "\u0080", "\u0301", "\u2028", "\ud800",
                          "\u0301A", "A \u0301B", "A\u00a0\u0301B"):
                with self.subTest(field=field, value=value):
                    document = json.loads(original)
                    owner = document if field == "entity" else document["agreements"][0]
                    owner[field] = value
                    path.write_text(json.dumps(document))
                    with self.assertRaisesRegex(ValueError, "text"):
                        review(self.source)

    def test_csv_evidence_and_ids_refuse_invisible_controls(self):
        for name, field in (("ledger.csv", "evidence"), ("allocations.csv", "evidence"),
                            ("receipts.csv", "evidence"), ("ledger.csv", "ledger_id")):
            original = (self.source / name).read_bytes()
            for value in ("\u200b", "\u0080", "\u202e", "\u0301", "\u0301A", "A \u0301B"):
                with self.subTest(name=name, field=field, value=value):
                    self.edit_csv(name, lambda rows: rows[0].update({field: value}))
                    with self.assertRaises(ValueError):
                        review(self.source)
                    (self.source / name).write_bytes(original)

    def test_valid_unicode_labels_and_evidence_are_preserved(self):
        path = self.source / "agreements.json"
        document = json.loads(path.read_text())
        document["entity"] = "Caf\u00e9 Community \u4e2d\u6587"
        document["agreements"][0]["agreement_evidence"] = "Cafe\u0301\u0308 reviewed agreement 1"
        path.write_text(json.dumps(document))
        result = review(self.source)
        self.assertEqual(result["entity"], document["entity"])
        self.assertEqual(result["allocation_evidence"][0]["agreement_evidence"], document["agreements"][0]["agreement_evidence"])
        output = self.root / "unicode"
        write(result, output)
        self.assertEqual(json.loads((output / "workpaper.json").read_text())["entity"], document["entity"])

    def test_concurrent_destination_is_not_replaced(self):
        output = self.root / "output"
        publish = grant_workpaper.publish_directory
        identity = []
        def create_then_publish(staged, destination):
            destination.mkdir()
            identity.append(destination.stat().st_ino)
            publish(staged, destination)
        with patch.object(grant_workpaper, "publish_directory", create_then_publish):
            with self.assertRaises(FileExistsError):
                write(review(self.source), output)
        self.assertEqual(output.stat().st_ino, identity[0])
        self.assertEqual(list(output.iterdir()), [])
        self.assertEqual(list(self.root.glob(".grant-workpaper-*")), [])

    def test_publish_refuses_existing_file_and_symlink(self):
        staged = self.root / "staged"
        staged.mkdir()
        (staged / "evidence").write_text("complete")
        destination = self.root / "existing"
        destination.write_text("original")
        with self.assertRaises(FileExistsError):
            grant_workpaper.publish_directory(staged, destination)
        self.assertEqual(destination.read_text(), "original")
        self.assertTrue((staged / "evidence").is_file())
        link = self.root / "link"
        try:
            link.symlink_to(destination)
        except OSError as exc:
            self.skipTest(f"Symlink creation unavailable: {exc}")
        with self.assertRaises(FileExistsError):
            grant_workpaper.publish_directory(staged, link)
        self.assertTrue(link.is_symlink())
        self.assertEqual(destination.read_text(), "original")

    def test_publish_failure_leaves_no_partial_output(self):
        output = self.root / "output"
        with patch.object(grant_workpaper, "publish_directory", side_effect=OSError("Unsupported filesystem")):
            with self.assertRaisesRegex(OSError, "Unsupported filesystem"):
                write(review(self.source), output)
        self.assertFalse(output.exists())
        self.assertEqual(list(self.root.glob(".grant-workpaper-*")), [])

    def test_markdown_labels_are_literal(self):
        result = review(self.source)
        result["entity"] = "[Fabricated](https://example.invalid) *entity*"
        output = self.root / "output"
        write(result, output)
        markdown = (output / "workpaper.md").read_text()
        self.assertIn(r"\[Fabricated\]\(https://example\.invalid\) \*entity\*", markdown)

    def test_failed_write_leaves_no_final_pack_and_can_retry(self):
        result = review(self.source)
        output = self.root / "output"
        original = Path.write_text
        def fail_markdown(path, *args, **kwargs):
            if path.name == "workpaper.md":
                raise OSError("Fabricated disk failure")
            return original(path, *args, **kwargs)
        with patch.object(Path, "write_text", fail_markdown):
            with self.assertRaisesRegex(OSError, "disk failure"):
                write(result, output)
        self.assertFalse(output.exists())
        self.assertEqual(list(self.root.glob(".grant-workpaper-*")), [])
        write(result, output)
        self.assertEqual(json.loads((output / "workpaper.json").read_text()), result)


if __name__ == "__main__":
    unittest.main()
