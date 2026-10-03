import csv
import hashlib
import importlib.util
import io
import json
import subprocess  # nosec B404 - real CLI integration checks use fixed local code.
import sys
import tempfile
import unittest
from decimal import Decimal, localcontext
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("aged_review", ROOT / "tools/xero_aged_receivables.py")
if spec is None or spec.loader is None:
    raise ImportError("Cannot load the aged receivables review tool.")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def csv_bytes(records):
    output = io.StringIO(newline="")
    csv.writer(output).writerows(records)
    return output.getvalue().encode()


class AgedReceivablesReviewTests(unittest.TestCase):
    def setUp(self):
        self.content = (ROOT / "samples/sample-aged-receivables-review.csv").read_bytes()
        self.manifest = json.loads((ROOT / "samples/sample-aged-receivables-manifest.json").read_text())
        self.control = json.loads((ROOT / "samples/sample-aged-receivables-control.json").read_text())

    def review(self, content=None, **changes):
        content = self.content if content is None else content
        manifest = {**self.manifest, "source_sha256": hashlib.sha256(content).hexdigest(), **changes}
        return module.inspect_export(content, manifest, self.control)

    def test_three_way_tie_preserves_duplicate_labels_credit_and_total_contact(self):
        result = self.review()
        self.assertEqual(result["summary_status"], "PASS")
        self.assertEqual(result["debtor_decisions"], "REVIEW")
        self.assertEqual([row["Contact"] for row in result["rows"]], ["00123", "00123", "Total"])
        self.assertEqual([row["source_record"] for row in result["rows"]], [7, 8, 9])
        self.assertEqual(result["rows"][1]["Total"], "-25")
        self.assertTrue(all(tie["agrees"] for tie in result["ties"]))
        self.assertEqual(result["ties"][-1]["left"], "85")
        columns = {tie["check"]: tie["left"] for tie in result["ties"]
                   if tie["check"].startswith("contact column ")}
        self.assertEqual(columns, {
            "contact column Current to export footer Current": "100",
            "contact column < 1 Month to export footer < 1 Month": "-25",
            "contact column 1 Month to export footer 1 Month": "10",
            "contact column 2 Months to export footer 2 Months": "0",
            "contact column 3 Months to export footer 3 Months": "0",
            "contact column Older to export footer Older": "0",
        })

    def test_permutations_split_credit_and_reversed_signs_preserve_supported_totals(self):
        import itertools

        records = list(csv.reader(io.StringIO(self.content.decode())))
        for population in itertools.permutations(records[6:9]):
            result = self.review(csv_bytes(records[:6] + list(population) + records[9:]))
            self.assertEqual(result["summary_status"], "PASS")
            self.assertEqual(len(result["rows"]), 3)
            self.assertEqual([row["source_record"] for row in result["rows"]], [7, 8, 9])
            self.assertEqual(result["debtor_decisions"], "REVIEW")
        split = [row[:] for row in records]
        first, second = split[7][:], split[7][:]
        first[2], first[-1] = "-10", "-10"
        second[2], second[-1] = "-15", "-15"
        split[7:8] = [first, second]
        result = self.review(csv_bytes(split), footer_record=11, on_screen_rows=4)
        self.assertEqual(result["summary_status"], "PASS")
        self.assertEqual([row["Total"] for row in result["rows"]], ["100", "-10", "-15", "10"])
        self.assertEqual(len(result["ties"]), 14)
        negated = [row[:] for row in records]
        for row in negated[6:]:
            for index in range(1, len(row)):
                row[index] = str(-Decimal(row[index] or "0"))
        content = csv_bytes(negated)
        manifest = {**self.manifest, "source_sha256": hashlib.sha256(content).hexdigest(), "on_screen_total": "-85"}
        result = module.inspect_export(content, manifest, {**self.control, "amount": "-85"})
        self.assertEqual(result["summary_status"], "PASS")
        self.assertEqual([row["Total"] for row in result["rows"]], ["-100", "25", "-10"])

    def test_optional_current_bom_quoted_commas_and_newlines(self):
        records = list(csv.reader(io.StringIO(self.content.decode())))
        records[5].remove("Current")
        for row in records[6:]:
            row.pop(1)
        records[6][1] = "100"
        records[6][0] = " 001, Café\nBranch "
        records[-1][1] = "75"
        out = io.StringIO(newline="")
        csv.writer(out).writerows(records)
        result = self.review(b"\xef\xbb\xbf" + out.getvalue().encode())
        self.assertEqual(result["summary_status"], "PASS")
        self.assertEqual(result["rows"][0]["Contact"], " 001, Café\nBranch ")
        columns = [tie for tie in result["ties"] if tie["check"].startswith("contact column ")]
        self.assertEqual(len(columns), 5)
        self.assertFalse(any("Current" in tie["check"] for tie in columns))
        records[-1][1] = "65"
        records[-1][-2] = "10"
        failed = self.review(csv_bytes(records))
        self.assertEqual(failed["summary_status"], "REVIEW")
        self.assertEqual([tie["check"] for tie in failed["ties"] if not tie["agrees"]], [
            "contact column < 1 Month to export footer < 1 Month",
            "contact column Older to export footer Older",
        ])

    def test_offsetting_footer_errors_are_reported_for_every_ageing_column(self):
        baseline = self.review()
        records = list(csv.reader(io.StringIO(self.content.decode())))
        for name in records[5][1:-1]:
            with self.subTest(column=name):
                changed = [row[:] for row in records]
                other = "Older" if name != "Older" else "Current"
                index, other_index = records[5].index(name), records[5].index(other)
                changed[-1][index] = str(Decimal(changed[-1][index]) - 10)
                changed[-1][other_index] = str(Decimal(changed[-1][other_index]) + 10)
                result = self.review(csv_bytes(changed))
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertEqual(result["debtor_decisions"], "REVIEW")
                self.assertEqual(result["rows"], baseline["rows"])
                failed = {tie["check"] for tie in result["ties"] if not tie["agrees"]}
                self.assertEqual(failed, {
                    f"contact column {name} to export footer {name}",
                    f"contact column {other} to export footer {other}",
                })
                self.assertEqual(len(result["exceptions"]), 2)
                old_ties = [tie for tie in result["ties"]
                            if not tie["check"].startswith("contact column ")]
                self.assertEqual(len(old_ties), 7)
                self.assertTrue(all(tie["agrees"] for tie in old_ties))

    def test_column_ties_keep_both_signed_half_cent_boundaries(self):
        records = list(csv.reader(io.StringIO(self.content.decode())))
        for text, agrees in (("0.0049", True), ("0.005", False), ("-0.005", False)):
            with self.subTest(delta=text):
                delta = Decimal(text)
                changed = [row[:] for row in records]
                changed[-1][1] = str(Decimal("100") - delta)
                changed[-1][-2] = str(delta)
                result = self.review(csv_bytes(changed))
                self.assertEqual(result["summary_status"], "PASS" if agrees else "REVIEW")
                ties = {tie["check"]: tie for tie in result["ties"]}
                current = ties["contact column Current to export footer Current"]
                older = ties["contact column Older to export footer Older"]
                self.assertEqual(current["difference"], str(delta))
                self.assertEqual(older["difference"], str(-delta))
                self.assertEqual((current["agrees"], older["agrees"]), (agrees, agrees))
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]
                                    if not tie["check"].startswith("contact column ")))

    def test_unsupported_titles_do_not_skip_arithmetic_or_preserved_rows(self):
        baseline = self.review()
        for title in ("Aged Payables Summary", "Unrelated Summary", "Aged Receivables Detail",
                      "Aged Receivables Summary - Custom", "aged receivables summary"):
            with self.subTest(title=title):
                source = self.content.replace(b"Aged Receivables Summary", title.encode(), 1)
                result = self.review(source, report_name=title)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertEqual(result["debtor_decisions"], "REVIEW")
                self.assertEqual(result["rows"], baseline["rows"])
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))
                self.assertEqual(result["source_sha256"], result["manifest"]["source_sha256"])
                self.assertEqual(result["exceptions"], [
                    "The first CSV record does not declare the supported report title.",
                    "Manifest report_name does not declare the supported report title.",
                ])

    def test_source_and_manifest_titles_are_checked_independently(self):
        for source_title, manifest_title, expected in (
            ("Unrelated Summary", "Aged Receivables Summary", "first CSV record"),
            ("Aged Receivables Summary", "Aged Payables Summary", "Manifest report_name"),
            ("Aged Receivables Summary", " Aged Receivables Summary ", "Manifest report_name"),
            ("", "Aged Receivables Summary", "first CSV record"),
        ):
            with self.subTest(source=source_title, manifest=manifest_title):
                source = self.content.replace(b"Aged Receivables Summary", source_title.encode(), 1)
                result = self.review(source, report_name=manifest_title)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertEqual(len(result["exceptions"]), 1)
                self.assertIn(expected, result["exceptions"][0])
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))
        source = self.content.replace(b"Aged Receivables Summary", b" Aged Receivables Summary ", 1)
        self.assertEqual(self.review(source)["summary_status"], "PASS")

    def test_canonical_title_in_other_metadata_cannot_rescue_first_record(self):
        self.manifest["entity"] = "Aged Receivables Summary"
        self.control["entity"] = "Aged Receivables Summary"
        for title in (b"Unrelated Summary", b""):
            with self.subTest(title=title):
                source = self.content.replace(b"Aged Receivables Summary", title, 1)
                source = source.replace(b"Synthetic Entity A", b"Aged Receivables Summary", 1)
                result = self.review(source)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertEqual(result["exceptions"], [
                    "The first CSV record does not declare the supported report title."])
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))

    def test_large_signed_amounts_remain_exact_under_low_ambient_precision(self):
        records = list(csv.reader(io.StringIO(self.content.decode())))
        records[6][1] = records[6][-1] = "999999999999999.9999"
        records[7][2] = records[7][-1] = "-999999999999999.9998"
        records[-1][1] = "999999999999999.9999"
        records[-1][2] = "-999999999999999.9998"
        records[-1][-1] = "10.0001"
        self.control["amount"] = "10.0001"
        with localcontext() as context:
            context.prec = 6
            result = self.review(csv_bytes(records), on_screen_total="10.0001")
            self.assertEqual(context.prec, 6)
        self.assertEqual(result["summary_status"], "PASS")
        ties = {tie["check"]: tie for tie in result["ties"]}
        self.assertEqual(ties["contact column Current to export footer Current"]["left"],
                         "999999999999999.9999")
        self.assertEqual(ties["contact column < 1 Month to export footer < 1 Month"]["left"],
                         "-999999999999999.9998")
        self.assertEqual(ties["contact totals to export footer"]["left"], "10.0001")
        self.assertTrue(all(tie["agrees"] for tie in result["ties"]))

    def test_other_metadata_cannot_masquerade_as_the_source_entity(self):
        for entity in ("Aged Receivables Summary", "As at 30 September 2026",
                       "Ageing by due date", "Synthetic Entity B"):
            with self.subTest(entity=entity):
                manifest = {**self.manifest, "entity": entity}
                control = {**self.control, "entity": entity}
                result = module.inspect_export(self.content, manifest, control)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertEqual(result["debtor_decisions"], "REVIEW")
                self.assertTrue(any("contradictory entity" in issue for issue in result["exceptions"]))
                self.assertNotIn("export footer to receivables control",
                                 [tie["check"] for tie in result["ties"]])
                self.assertEqual(len(result["rows"]), 3)
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))

    def test_missing_duplicate_or_contradictory_entity_records_need_review(self):
        for replacement, footer_record in ((b"", 10),
                                           (b"Synthetic Entity A\nSynthetic Entity A", 11),
                                           (b"Synthetic Entity A\nSynthetic Entity B", 11)):
            with self.subTest(entity_records=replacement):
                source = self.content.replace(b"Synthetic Entity A", replacement, 1)
                result = self.review(source, footer_record=footer_record)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertTrue(any("contradictory entity" in issue for issue in result["exceptions"]))
                self.assertNotIn("export footer to receivables control",
                                 [tie["check"] for tie in result["ties"]])
                self.assertEqual([row["Contact"] for row in result["rows"]], ["00123", "00123", "Total"])
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))

    def test_control_tie_requires_source_confirmed_cutoff(self):
        original = b"As at 30 September 2026"
        for replacement, footer_record in ((b"As at 31 August 2026", 10),
                                           (b"", 10),
                                           (original + b"\n" + original, 11)):
            with self.subTest(date_records=replacement):
                result = self.review(self.content.replace(original, replacement, 1),
                                     footer_record=footer_record)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertNotIn("export footer to receivables control",
                                 [tie["check"] for tie in result["ties"]])
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))
        manifest = {**self.manifest, "as_at": "2026-08-31"}
        control = {**self.control, "as_at": "2026-08-31"}
        result = module.inspect_export(self.content, manifest, control)
        self.assertEqual(result["summary_status"], "REVIEW")
        self.assertNotIn("export footer to receivables control",
                         [tie["check"] for tie in result["ties"]])

    def test_distinct_entity_record_can_move_within_preheader_metadata(self):
        records = list(csv.reader(io.StringIO(self.content.decode())))
        records[1], records[2] = records[2], records[1]
        result = self.review(csv_bytes(records))
        self.assertEqual(result["summary_status"], "PASS")
        self.assertEqual(result["debtor_decisions"], "REVIEW")
        self.assertEqual(len(result["ties"]), 13)
        self.assertTrue(all(tie["agrees"] for tie in result["ties"]))

    def test_cli_preserves_pass_review_and_invalid_input_exits(self):
        columns = self.content.replace(b"Total,100,-25,10,0,0,0,85",
                                       b"Total,90,-25,10,0,0,10,85")
        cases = [
            (self.content, "Aged Receivables Summary", 0, "PASS"),
            (self.content.replace(b"Aged Receivables Summary", b"Aged Payables Summary"),
             "Aged Payables Summary", 2, "REVIEW"),
            (self.content.replace(b"Aged Receivables Summary", b"Unrelated Summary"),
             "Unrelated Summary", 2, "REVIEW"),
            (columns, "Aged Receivables Summary", 2, "REVIEW"),
            (self.content.replace(b"00123,100,,0,0,0,0,100", b"00123,100"),
             "Aged Receivables Summary", 1, "INVALID_INPUT"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            source, manifest, control = (Path(directory) / name for name in ("source.csv", "manifest.json", "control.json"))
            control.write_text(json.dumps(self.control), encoding="utf-8")
            for content, title, exit_code, status in cases:
                with self.subTest(status=status, title=title):
                    source.write_bytes(content)
                    evidence = {**self.manifest, "report_name": title,
                                "source_sha256": hashlib.sha256(content).hexdigest()}
                    manifest.write_text(json.dumps(evidence), encoding="utf-8")
                    # Fixed interpreter and reviewed script; fabricated file paths are list arguments.
                    completed = subprocess.run([  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit
                        sys.executable, "-B", str(ROOT / "tools/xero_aged_receivables.py"),
                        str(source), "--manifest", str(manifest), "--control", str(control),
                    ], capture_output=True, text=True, check=False)
                    self.assertEqual(completed.returncode, exit_code, completed.stderr)
                    result = json.loads(completed.stderr if exit_code == 1 else completed.stdout)
                    self.assertEqual(result["summary_status"], status)
                    if exit_code != 1:
                        self.assertEqual(result["debtor_decisions"], "REVIEW")

    def test_blank_is_zero_but_short_extra_and_malformed_rows_are_invalid(self):
        self.assertEqual(self.review()["rows"][0]["< 1 Month"], "0")
        for value in (b"00123,100,0,0,0,0,100", b"00123,100,,0,0,0,0,100,extra",
                      b'"unterminated,100,,0,0,0,0,100'):
            with self.subTest(value=value), self.assertRaises((ValueError, csv.Error)):
                self.review(self.content.replace(b"00123,100,,0,0,0,0,100", value))

    def test_invalid_amounts_and_headers_are_refused(self):
        for value in ("NaN", "Infinity", "-Infinity", "TBC", "1e3", "0.00001", "(25.00)", "1,00"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.review(self.content.replace(b"00123,100,", f"00123,{value},".encode()))
        for value in (b"Contact,Current,< 1 Month,1 Month,2 Months,3 Months,Total,Total",
                      b"Contact,Customer,< 1 Month,1 Month,2 Months,3 Months,Older,Total"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.review(self.content.replace(b"Contact,Current,< 1 Month,1 Month,2 Months,3 Months,Older,Total", value))

    def test_half_cent_boundary_uses_signed_unrounded_differences(self):
        for delta, agrees in (("100.0049", True), ("100.005", False), ("99.995", False)):
            with self.subTest(delta=delta):
                result = self.review(self.content.replace(b"00123,100,", f"00123,{delta},".encode()))
                self.assertEqual(result["ties"][0]["agrees"], agrees)
                self.assertEqual(result["summary_status"], "PASS" if agrees else "REVIEW")

    def test_three_differences_are_kept_separate(self):
        self.control["amount"] = "84"
        result = self.review(self.content.replace(b"00123,100,,0,0,0,0,100", b"00123,100,,0,0,0,0,101"))
        failed = [tie["check"] for tie in result["ties"] if not tie["agrees"]]
        self.assertEqual(failed, ["record 7 buckets to total", "contact totals to export footer",
                                  "export footer to receivables control"])

    def test_missing_control_and_mismatched_metadata_remain_review(self):
        self.assertEqual(module.inspect_export(self.content, self.manifest)["summary_status"], "REVIEW")
        for key, value in (("entity", "Synthetic Entity B"), ("as_at", "2026-08-31"),
                           ("currency", "NZD"), ("filters", "one tracking category"),
                           ("population", "all assets"), ("accounting_basis", "cash"),
                           ("sign_convention", "credit-positive"), ("source", "")):
            with self.subTest(key=key):
                control = {**self.control, key: value}
                result = module.inspect_export(self.content, self.manifest, control)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertNotIn("export footer to receivables control", [tie["check"] for tie in result["ties"]])

    def test_missing_provenance_footer_or_screen_check_is_not_completion(self):
        for key in ("generated_at", "source_sha256", "on_screen_rows", "on_screen_total", "footer_record"):
            with self.subTest(key=key):
                manifest = dict(self.manifest)
                del manifest[key]
                result = module.inspect_export(self.content, manifest, self.control)
                self.assertEqual(result["summary_status"], "REVIEW")
                if key == "footer_record":
                    self.assertFalse(any(tie["check"].startswith("contact column ")
                                         for tie in result["ties"]))
        result = self.review(footer_record=999)
        self.assertEqual(result["summary_status"], "REVIEW")
        self.assertFalse(any(tie["check"].startswith("contact column ") for tie in result["ties"]))
        with self.assertRaises(ValueError):
            self.review(footer_record=7)
        with self.assertRaises(ValueError):
            self.review(footer_record=True)

    def test_duplicate_json_fields_are_invalid(self):
        with self.assertRaises(ValueError):
            json.loads('{"amount":"1","amount":"2"}', object_pairs_hook=module.unique_object)

    def test_record_and_screen_counts_require_builtin_integers(self):
        class Count(int):
            pass

        for value in (True, False, Count(self.manifest["footer_record"]), "10", 10.0):
            with self.subTest(footer_record=value), self.assertRaises(ValueError):
                self.review(footer_record=value)
        for value in (True, False, Count(3), "3", 3.0, -1):
            with self.subTest(on_screen_rows=value):
                result = self.review(on_screen_rows=value)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertEqual(result["exceptions"], [
                    "On-screen contact row count is missing or does not agree."])
                self.assertEqual(result["debtor_decisions"], "REVIEW")

    def test_section_rows_and_incomplete_timestamp_or_screen_total_need_review(self):
        content = self.content.replace(b"00123,100,,0,0,0,0,100", b"Section A,,,,,,,")
        result = self.review(content)
        self.assertEqual(result["summary_status"], "REVIEW")
        self.assertEqual(result["rows"][0]["Contact"], "Section A")
        self.assertTrue(any("sectioned exports" in issue for issue in result["exceptions"]))
        for key, value in (("generated_at", "2026-10-01"), ("generated_at", "2026-10-01T09:00:00"),
                           ("on_screen_total", "")):
            with self.subTest(key=key, value=value):
                self.assertEqual(self.review(**{key: value})["summary_status"], "REVIEW")

    def test_duplicate_or_contradictory_ageing_metadata_never_passes(self):
        original = f"Ageing by {self.manifest['ageing_basis']}".encode()
        self.assertIn(original, self.content)
        for extra in (original, b"Ageing by Invoice Date"):
            with self.subTest(extra=extra):
                content = self.content.replace(original, original + b"\n" + extra)
                result = self.review(content, footer_record=11)
                self.assertEqual(result["summary_status"], "REVIEW")
                self.assertTrue(any("contradictory ageing basis" in issue for issue in result["exceptions"]))
                self.assertEqual([row["Contact"] for row in result["rows"]], ["00123", "00123", "Total"])
                self.assertEqual(result["rows"][1]["Total"], "-25")
                self.assertTrue(all(tie["agrees"] for tie in result["ties"]))


if __name__ == "__main__":
    unittest.main()
