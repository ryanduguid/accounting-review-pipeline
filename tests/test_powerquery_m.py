"""Static syntax and fixture checks for TMDL-embedded Power Query expressions."""

from __future__ import annotations

import csv
import re
import unittest
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
EXPRESSIONS_FILE = (
    BASE_DIR
    / "australian-accounting-power-bi.SemanticModel"
    / "definition"
    / "expressions.tmdl"
)
SAMPLES_DIR = BASE_DIR / "samples"


def named_expressions() -> dict[str, str]:
    """Return the fenced M source for each named TMDL expression."""
    content = EXPRESSIONS_FILE.read_text(encoding="utf-8")
    expressions: dict[str, str] = {}
    pattern = re.compile(
        r"^expression\s+(?P<name>'[^']+'|[^\s=]+)\s*=\s*```[\t ]*\r?\n"
        r"(?P<body>.*?)(?=^[\t ]*```[\t ]*$)",
        re.MULTILINE | re.DOTALL,
    )

    for match in pattern.finditer(content):
        name = match.group("name").strip("'")
        if name in expressions:
            raise AssertionError(f"Duplicate named expression: {name}")
        expressions[name] = match.group("body")

    return expressions


class TestPowerQueryM(unittest.TestCase):
    def test_expected_named_expressions_are_embedded_in_tmdl(self) -> None:
        self.assertTrue(EXPRESSIONS_FILE.is_file(), "expressions.tmdl must exist")
        self.assertEqual(
            sorted(named_expressions()),
            [
                "Dim_Account",
                "Dim_Date_AU",
                "Dim_Entity",
                "Fact_ATOBenchmark",
                "Fact_Budget",
                "Fact_GeneralLedger",
                "Fact_PayrollSuper",
                "Fx_ValidateABN",
            ],
        )

    def test_named_expressions_have_balanced_let_in_blocks(self) -> None:
        """Every M script must have balanced let ... in blocks."""
        for name, content in named_expressions().items():
            with self.subTest(expression=name):
                let_count = len(re.findall(r"^[\t ]*let[\t ]*$", content, re.MULTILINE))
                in_count = len(re.findall(r"^[\t ]*in[\t ]*$", content, re.MULTILINE))
                self.assertGreaterEqual(let_count, 1, f"{name} missing 'let'")
                self.assertEqual(let_count, in_count, f"{name} has unbalanced let/in blocks")

    def test_abn_validator_m_logic(self) -> None:
        """Verify the ABN expression uses the statutory weights and Modulus 89."""
        content = named_expressions()["Fx_ValidateABN"]
        self.assertIn("10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19", content)
        self.assertIn("Number.Mod", content)
        self.assertIn("89", content)

    def test_sample_entity_abns_pass_the_shipped_checksum(self) -> None:
        """Run Fx_ValidateABN's own weights over every ABN in the entity master.

        Shipping an entity the model's own validator rejects flags a fabricated group
        entity as having an invalid ABN the moment the function is wired up.
        """
        weights_source = re.search(
            r"Weights = \{([^}]*)\}", named_expressions()["Fx_ValidateABN"]
        )
        self.assertIsNotNone(weights_source, "Fx_ValidateABN must declare its weights")
        weights = [int(weight) for weight in weights_source.group(1).split(",")]  # type: ignore[union-attr]
        self.assertEqual(len(weights), 11)

        rejected: list[str] = []
        with (SAMPLES_DIR / "sample-entities.csv").open(
            newline="", encoding="utf-8"
        ) as handle:
            for row in csv.DictReader(handle):
                digits = [int(char) for char in row["ABN"] if char.isdigit()]
                if len(digits) != 11:
                    rejected.append(f"{row['EntityID']}:{row['ABN']}")
                    continue
                digits[0] -= 1
                if sum(d * w for d, w in zip(digits, weights)) % 89 != 0:
                    rejected.append(f"{row['EntityID']}:{row['ABN']}")

        self.assertEqual(
            rejected,
            [],
            "sample-entities.csv holds ABNs that fail the modulus-89 checksum",
        )

    def test_csv_source_column_counts_match_fixture_rows(self) -> None:
        cases = {
            "Dim_Account": "sample-chart-of-accounts.csv",
            "Dim_Entity": "sample-entities.csv",
            "Fact_ATOBenchmark": "sample-ato-benchmarks.csv",
            "Fact_Budget": "sample-budgets.csv",
            "Fact_GeneralLedger": "sample-general-ledger.csv",
            "Fact_PayrollSuper": "sample-payroll-super.csv",
        }
        expressions = named_expressions()

        for query_name, sample_name in cases.items():
            with self.subTest(query=query_name):
                content = expressions[query_name]
                match = re.search(r"\bColumns=(\d+)\b", content)
                self.assertIsNotNone(match, f"{query_name} must declare its CSV width")
                declared_columns = int(match.group(1))  # type: ignore[union-attr]

                with (SAMPLES_DIR / sample_name).open(
                    newline="", encoding="utf-8"
                ) as handle:
                    fixture_widths = {len(row) for row in csv.reader(handle)}

                self.assertEqual(
                    fixture_widths,
                    {declared_columns},
                    f"{query_name} declares {declared_columns} columns but "
                    f"{sample_name} has row widths {sorted(fixture_widths)}",
                )

    def test_dim_account_preserves_quoted_account_name(self) -> None:
        content = named_expressions()["Dim_Account"]
        self.assertIn("QuoteStyle=QuoteStyle.Csv", content)

        with (SAMPLES_DIR / "sample-chart-of-accounts.csv").open(
            newline="", encoding="utf-8"
        ) as handle:
            account = next(row for row in csv.reader(handle) if row[0] == "200")

        self.assertEqual(len(account), 9)
        self.assertEqual(account[1], "Property, Plant & Equipment")


if __name__ == "__main__":
    unittest.main()
