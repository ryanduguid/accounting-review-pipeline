"""Contract checks for the committed Power Query sample.

The sample is only useful while it agrees with the CSV the exporter actually
writes. These tests pin that agreement, the typing decisions that make a trial
balance survive the trip into Power BI, and the README link that leads people
to it.
"""

from __future__ import annotations

import csv
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
QUERY_PATH = ROOT / "samples" / "power-bi-query.pq"
SAMPLE_PATH = ROOT / "samples" / "sample-output.csv"
README_PATH = ROOT / "README.md"

QUERY = QUERY_PATH.read_text(encoding="utf-8")

TEXT_COLUMNS = ("Tenant", "Section", "AccountID", "AccountName", "AccountCode")
MONEY_COLUMNS = ("Debit", "Credit", "YTDDebit", "YTDCredit")


def _declared_columns() -> list[str]:
    """The ExpectedColumns list the query fails closed against."""
    block = re.search(r"ExpectedColumns = \{(.*?)\}", _code_only(), re.DOTALL)
    assert block is not None, "the query must declare ExpectedColumns"
    return re.findall(r'"([^"]+)"', block.group(1))


def _probe_margin() -> int:
    """How many columns past the contract the query reads and checks."""
    match = re.search(r"ProbeMargin = (\d+)", _code_only())
    assert match is not None, "the query must declare ProbeMargin"
    return int(match.group(1))


def _assigned_types() -> dict[str, str]:
    """Every {"Column", type} pair in the Table.TransformColumnTypes step."""
    block = re.search(r"Table\.TransformColumnTypes\s*\(\s*[^,]+,\s*\{((?:[^{}]|\{[^{}]*\})*)\}", _code_only())
    assert block is not None, "the query must transform its column types"
    return {
        name: assigned.strip()
        for name, assigned in re.findall(r'\{"(\w+)",\s*([^}]+)\}', block.group(1))
    }


def _code_only() -> str:
    """The query with its `//` commentary removed.

    The comments talk about tokens and credentials to explain their absence,
    so a scan for those words has to read the code rather than the prose.
    """
    return re.sub(r'"(?:[^"]|"")*"|//[^\r\n]*',
                  lambda match: "" if match.group().startswith("//") else match.group(), QUERY)



def _sample_header() -> list[str]:
    with SAMPLE_PATH.open(encoding="utf-8-sig", newline="") as handle:
        return list(csv.DictReader(handle).fieldnames or ())


class PowerBiSampleTests(unittest.TestCase):
    def test_declared_columns_match_the_committed_sample_header(self) -> None:
        """The whole point of the sample: if the exporter's shape changes, this
        query is wrong and must be updated with it."""
        self.assertEqual(_declared_columns(), _sample_header())

    def test_every_column_gets_an_explicit_type(self) -> None:
        self.assertEqual(sorted(_assigned_types()), sorted(_sample_header()))

    def test_the_report_date_is_a_date(self) -> None:
        self.assertEqual(_assigned_types()["ReportDate"], "type date")

    def test_identifiers_and_codes_stay_text(self) -> None:
        """AccountCode is the one that bites: 090 is not 90."""
        types = _assigned_types()
        for column in TEXT_COLUMNS:
            self.assertEqual(types[column], "type text", column)

    def test_money_columns_use_a_fixed_decimal(self) -> None:
        types = _assigned_types()
        for column in MONEY_COLUMNS:
            self.assertEqual(types[column], "Currency.Type", column)

    def test_the_committed_sample_really_has_a_leading_zero_code(self) -> None:
        """Guards the reason AccountCode is text, not just the decision."""
        with SAMPLE_PATH.open(encoding="utf-8-sig", newline="") as handle:
            codes = [row["AccountCode"] for row in csv.DictReader(handle)]
        self.assertTrue(
            any(code.startswith("0") and len(code) > 1 for code in codes),
            "the fabricated sample no longer demonstrates a leading-zero code",
        )

    def test_the_parse_is_wider_than_the_contract(self) -> None:
        """Csv.Document normalises to the column count it is handed, dropping
        extra fields and padding short rows. Asking for exactly 10 would
        reshape a malformed file into the expected shape before the header
        check could see it, so the query reads several columns wider and treats
        anything past the contract as proof the file is too wide. One extra
        column was not enough: a row whose surplus data sat past an empty
        eleventh field read as well formed."""
        code = _code_only()
        self.assertRegex(
            code,
            r'Csv\.Document\(\s*File\.Contents\(SourcePath\),\s*'
            r'List\.Count\(ExpectedColumns\) \+ ProbeMargin,\s*",",\s*ExtraValues\.Error,\s*65001\s*\)',
        )
        self.assertGreater(_probe_margin(), 1)
        self.assertNotIn("Columns = List.Count(ExpectedColumns),", code)
        self.assertIn("Probe = Table.Buffer(Csv.Document(", code)

    def test_each_malformed_shape_has_its_own_refusal(self) -> None:
        code = _code_only()
        for reason in (
            "carries more than the ten columns",
            "does not have the ten columns",
            "is missing a field export_tb.py always writes",
        ):
            self.assertIn(reason, code)
        self.assertEqual(code.count("error Error.Record("), 3)

    def test_the_typed_step_reads_the_fully_checked_table(self) -> None:
        """Every refusal has to sit upstream of the types, or it is decorative."""
        code = _code_only()
        self.assertIn("Table.TransformColumnTypes(\n        Complete,", code)

    def test_a_blank_trailing_line_is_not_treated_as_a_damaged_row(self) -> None:
        self.assertIn("List.IsEmpty(Present(Record.FieldValues(_)))", _code_only())

    def test_padding_is_recognised_however_the_host_engine_spells_it(self) -> None:
        """The refusal has to read padding, not one host's spelling of it.

        Csv.Document pads a short record to the requested column count. Power
        BI Desktop pads with null; Excel 16.0 build 20430 pads with empty
        text. While the query removed nulls only, the empty text in the
        eleventh column survived and an ordinary ten-column export was refused
        as being wider than ten columns.
        """
        code = _code_only()
        self.assertIn('IsAbsent = (value as any) as logical => value = null or value = ""', code)
        self.assertIn("List.Transform(OverflowColumns, (name) => Present(Table.Column(Probe, name)))", code)
        self.assertNotIn("List.RemoveNulls", code)
        self.assertNotIn("List.Contains(Record.FieldValues(_), null)", code)

    def test_the_width_rule_refuses_a_wide_file_and_accepts_the_committed_sample(self) -> None:
        """Port the query's width rule and run both host paddings through it.

        The M cannot be evaluated here, so the rule is reimplemented from the
        query's own ExpectedColumns count and ProbeMargin and applied to records
        parsed from the committed sample and from fabricated wide files. Native
        Power Query confirmation is a separate, native check.
        """
        expected = len(_declared_columns())
        margin = _probe_margin()

        def probe(records: list[list[str]], padding: object) -> list[object]:
            """Csv.Document with Columns = expected + margin, as each host pads."""
            overflow: list[object] = []
            for record in records:
                if len(record) > expected + margin:
                    raise ValueError("More fields than the parser's column count")
                widened = list(record[: expected + margin])
                widened += [padding] * (expected + margin - len(widened))
                overflow.extend(widened[expected:])
            return overflow

        def is_absent(value: object) -> bool:
            return value is None or value == ""

        with SAMPLE_PATH.open(encoding="utf-8-sig", newline="") as handle:
            sample = [row for row in csv.reader(handle) if row]
        self.assertTrue(all(len(row) == expected for row in sample))

        for padding in (None, ""):
            present = [v for v in probe(sample, padding) if not is_absent(v)]
            self.assertEqual(present, [], f"the committed sample is refused when padded with {padding!r}")

        wide = [row + ["surplus"] for row in sample]
        for padding in (None, ""):
            present = [v for v in probe(wide, padding) if not is_absent(v)]
            self.assertEqual(len(present), len(sample), "an 11-column file must still be refused")

        # The F445 case: the eleventh field is empty and the twelfth carries
        # data. Reading one extra column saw only the empty eleventh and loaded
        # the file as if it were well formed.
        skipped = [row + ["", "surplus"] for row in sample]
        for padding in (None, ""):
            present = [v for v in probe(skipped, padding) if not is_absent(v)]
            self.assertEqual(
                len(present),
                len(sample),
                "a file whose surplus data sits past an empty eleventh field must be refused",
            )
        narrow_probe = [row[: expected + 1][expected] for row in skipped]
        self.assertEqual(
            [v for v in narrow_probe if not is_absent(v)],
            [],
            "this is the case a single overflow column cannot see",
        )

        # The earlier defect: removing nulls alone kept every empty-text pad, so
        # under Excel's engine the committed sample looked wider than ten
        # columns in every row.
        null_only = [v for v in probe(sample, "") if v is not None]
        self.assertEqual(len(null_only), len(sample) * margin)

        beyond_probe = [row + [""] * margin + ["surplus"] for row in sample]
        for padding in (None, ""):
            with self.assertRaises(ValueError):
                probe(beyond_probe, padding)

    def test_the_source_path_is_an_unusable_placeholder(self) -> None:
        """A path the reader must replace, not one that quietly half-works."""
        self.assertIn("CHANGE-ME", QUERY)

    def test_the_query_reaches_no_further_than_a_local_file(self) -> None:
        """No Xero login: the sample loads a committed CSV and nothing else."""
        code = _code_only()
        for forbidden in ("Web.Contents", "OData.Feed", "client_id", "client_secret", "token"):
            self.assertNotIn(forbidden, code)
        self.assertIn("File.Contents", code)

    def test_readme_links_the_sample_from_the_power_bi_section(self) -> None:
        readme = README_PATH.read_text(encoding="utf-8")
        link = "[`samples/power-bi-query.pq`](samples/power-bi-query.pq)"
        self.assertIn(link, readme)
        self.assertLess(readme.index("## Power BI"), readme.index(link))
        self.assertLess(readme.index(link), readme.index("## Scheduled runs"))


if __name__ == "__main__":
    unittest.main()
