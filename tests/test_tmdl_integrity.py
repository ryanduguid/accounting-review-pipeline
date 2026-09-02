"""
test_tmdl_integrity.py - Validates TMDL definitions, star schema relationships, and measure hygiene.
"""

from __future__ import annotations

import csv
import json
import re
import unittest
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
PROJECT_NAME = "australian-accounting-power-bi"
PROJECT_FILE = BASE_DIR / f"{PROJECT_NAME}.pbip"
REPORT_DIR = BASE_DIR / f"{PROJECT_NAME}.Report"
SEMANTIC_MODEL_DIR = BASE_DIR / f"{PROJECT_NAME}.SemanticModel"
TMDL_DIR = SEMANTIC_MODEL_DIR / "definition"
TMDL_TABLES_DIR = TMDL_DIR / "tables"
RELATIONSHIPS_FILE = TMDL_DIR / "relationships.tmdl"
DATABASE_FILE = TMDL_DIR / "database.tmdl"
SAMPLES_DIR = BASE_DIR / "samples"

# The star schema drawn by README.md and docs/data-model.md, as (fromColumn, toColumn) pairs.
DOCUMENTED_RELATIONSHIPS = {
    ("Fact_GeneralLedger.AccountCode", "Dim_Account.AccountCode"),
    ("Fact_GeneralLedger.EntityID", "Dim_Entity.EntityID"),
    ("Fact_GeneralLedger.PostingDate", "Dim_Date.Date"),
    ("Fact_Budget.AccountCode", "Dim_Account.AccountCode"),
    ("Fact_Budget.EntityID", "Dim_Entity.EntityID"),
    ("Fact_Budget.PeriodDate", "Dim_Date.Date"),
    ("Fact_PayrollSuper.EntityID", "Dim_Entity.EntityID"),
    ("Fact_PayrollSuper.PayDate", "Dim_Date.Date"),
    ("Fact_PayrollSuper.EmployeeID", "Dim_Employee.EmployeeID"),
    ("Dim_Entity.ANZSIC_Code", "Dim_ANZSIC.ANZSIC_Code"),
    ("Fact_ATOBenchmark.ANZSIC_Code", "Dim_ANZSIC.ANZSIC_Code"),
}


def measure_expressions(tmdl_file: Path) -> dict[str, str]:
    """Map every measure in a TMDL table file to its DAX expression, flattened to one line."""
    lines = tmdl_file.read_text(encoding="utf-8").splitlines()
    object_starts = [
        index
        for index, line in enumerate(lines)
        if re.match(
            r"^\t(?:measure|column|partition|hierarchy|calculationGroup|annotation|///)",
            line,
        )
    ]
    expressions: dict[str, str] = {}

    for index, line in enumerate(lines):
        if not line.startswith("\tmeasure "):
            continue

        declaration = line.removeprefix("\tmeasure ")
        name = declaration.split(" =", 1)[0].strip("'")
        next_object = next(
            (object_index for object_index in object_starts if object_index > index),
            len(lines),
        )
        # Keep the expression lines; drop trailing properties such as formatString and lineageTag.
        body = [declaration.split(" =", 1)[1]]
        body.extend(
            candidate
            for candidate in lines[index + 1 : next_object]
            if not re.match(r"^\t\t\w+:", candidate)
        )
        expressions[name] = " ".join(part.strip() for part in body if part.strip())

    return expressions


class TestTmdlIntegrity(unittest.TestCase):
    def test_project_path_graph_uses_one_canonical_identifier(self) -> None:
        project = json.loads(PROJECT_FILE.read_text(encoding="utf-8"))
        report = json.loads((REPORT_DIR / "definition.pbir").read_text(encoding="utf-8"))
        database = DATABASE_FILE.read_text(encoding="utf-8")

        self.assertEqual(
            project["artifacts"][0]["report"]["path"],
            f"{PROJECT_NAME}.Report",
        )
        self.assertEqual(
            report["datasetReference"]["byPath"]["path"],
            f"../{PROJECT_NAME}.SemanticModel",
        )
        self.assertIn(f"database '{PROJECT_NAME}'", database)

    def test_tables_directory_exists_and_populated(self) -> None:
        self.assertTrue(TMDL_TABLES_DIR.is_dir(), "TMDL tables directory must exist")
        tmdl_files = list(TMDL_TABLES_DIR.glob("*.tmdl"))
        self.assertGreaterEqual(len(tmdl_files), 8, "Expected at least 8 TMDL table definitions")

    def test_relationships_defined_and_valid(self) -> None:
        self.assertTrue(RELATIONSHIPS_FILE.is_file(), "relationships.tmdl must exist")
        content = RELATIONSHIPS_FILE.read_text(encoding="utf-8")

        rel_blocks = re.findall(
            r"relationship\s+'([^']+)'\s+fromColumn:\s+([\w\.]+)\s+toColumn:\s+([\w\.]+)",
            content,
        )
        self.assertGreaterEqual(len(rel_blocks), 5, "Expected at least 5 star schema relationships")

        for name, from_col, to_col in rel_blocks:
            self.assertIn(".", from_col, f"Relationship {name} fromColumn must include table name")
            self.assertIn(".", to_col, f"Relationship {name} toColumn must include table name")

    def test_documented_star_schema_edges_all_exist(self) -> None:
        """Every edge the README and data-model ER diagrams draw must be a real relationship.

        A documented edge that is missing leaves its dimension orphaned: filters never
        propagate and the dimension repeats the grand total on every row.
        """
        content = RELATIONSHIPS_FILE.read_text(encoding="utf-8")
        declared = set(
            re.findall(r"fromColumn:\s+([\w\.]+)\s+toColumn:\s+([\w\.]+)", content)
        )

        self.assertEqual(
            sorted(DOCUMENTED_RELATIONSHIPS - declared),
            [],
            "relationships.tmdl is missing a documented star schema edge",
        )
        self.assertEqual(
            sorted(declared - DOCUMENTED_RELATIONSHIPS),
            [],
            "relationships.tmdl declares an edge the ER diagrams do not document",
        )

    def test_dim_anzsic_declares_every_industry_code_the_model_joins_on(self) -> None:
        """Dim_ANZSIC is a literal lookup, so both ANZSIC relationships die without these codes."""
        partition = (TMDL_TABLES_DIR / "Dim_ANZSIC.tmdl").read_text(encoding="utf-8")
        declared = set(re.findall(r'\{"(\d{4})",', partition))

        joined: set[str] = set()
        for fixture in ("sample-entities.csv", "sample-ato-benchmarks.csv"):
            with (SAMPLES_DIR / fixture).open(newline="", encoding="utf-8") as handle:
                joined.update(row["ANZSIC_Code"] for row in csv.DictReader(handle))

        self.assertTrue(joined, "fixtures must supply the ANZSIC codes the model joins on")
        self.assertEqual(
            sorted(joined - declared),
            [],
            "Dim_ANZSIC must declare every ANZSIC code Dim_Entity and Fact_ATOBenchmark join on",
        )

    def test_ebitda_adds_back_depreciation_and_interest(self) -> None:
        """Depreciation (890) and finance interest (895) are in the Operating Expenses SubClass.

        Without an explicit add-back, EBITDA is the same arithmetic as Net Profit Before Tax.
        """
        expressions = measure_expressions(TMDL_TABLES_DIR / "Fact_GeneralLedger.tmdl")
        ebitda = expressions["EBITDA"]

        for account_code in ('"890"', '"895"'):
            self.assertIn(
                account_code,
                ebitda,
                f"EBITDA must add back account {account_code} to differ from Net Profit Before Tax",
            )

    def test_balance_sheet_check_accounts_for_earnings_not_closed_to_equity(self) -> None:
        """Current-period earnings are never closed to account 510 in the ledger fixture.

        Total Equity therefore excludes them, and a check of Assets less Liabilities and
        Equity alone returns the period's net profit rather than the documented $0.00, so
        it reads $0.00 only where that profit happens to be nil - in the fixture, only the
        single opening-balance day 2024-07-01.
        """
        expressions = measure_expressions(TMDL_TABLES_DIR / "Fact_GeneralLedger.tmdl")

        self.assertIn(
            "[Net Profit Before Tax]",
            expressions["Balance Sheet Check"],
            "Balance Sheet Check must include earnings to date or it returns the period's "
            "net profit instead of $0.00 wherever that profit is not nil",
        )

    def test_balance_sheet_measures_accumulate_to_the_last_date_in_context(self) -> None:
        """Balance sheet measures are described as cumulative and are plotted on a monthly axis.

        A plain class-filtered SUM returns only the period's movement under a date context.
        """
        expressions = measure_expressions(TMDL_TABLES_DIR / "Fact_GeneralLedger.tmdl")
        running_total = "DATESBETWEEN(Dim_Date[Date], BLANK(), MAX(Dim_Date[Date]))"
        expected_legs = {
            "Total Assets": 1,
            "Total Liabilities": 1,
            "Total Equity": 1,
            "Working Capital": 2,
        }

        for measure_name, legs in expected_legs.items():
            self.assertEqual(
                expressions[measure_name].count(running_total),
                legs,
                f"[{measure_name}] must accumulate all {legs} of its legs to the last visible date",
            )

    def test_benchmark_measures_follow_the_selected_entity_industry(self) -> None:
        """Dim_Entity filters one way into Dim_ANZSIC, so entity context never reaches the facts.

        Without TREATAS, every benchmark averages all industries at once and the risk
        rating becomes a constant.
        """
        expressions = measure_expressions(TMDL_TABLES_DIR / "Fact_ATOBenchmark.tmdl")
        industry_filter = "TREATAS(VALUES(Dim_Entity[ANZSIC_Code]), Fact_ATOBenchmark[ANZSIC_Code])"
        benchmark_measures = sorted(
            name for name in expressions if name.startswith("ATO Benchmark ")
        )

        self.assertEqual(len(benchmark_measures), 5)
        for measure_name in benchmark_measures:
            self.assertIn(
                industry_filter,
                expressions[measure_name],
                f"[{measure_name}] must be scoped to the industries of the entities in context",
            )

    def test_compliance_rate_is_blank_when_no_super_was_accrued(self) -> None:
        """A period with no payroll events is missing data, not full statutory compliance."""
        expressions = measure_expressions(TMDL_TABLES_DIR / "Fact_PayrollSuper.tmdl")
        compliance_rate = expressions["Payday Super Compliance Rate %"]

        self.assertIn("BLANK()", compliance_rate)
        self.assertNotIn(
            "1.0",
            compliance_rate,
            "A zero super liability must not render as 100% compliance",
        )

    def test_all_measures_have_supported_descriptions_and_numeric_formats(self) -> None:
        """Require descriptions for every measure and formats for non-text measures."""
        tmdl_files = list(TMDL_TABLES_DIR.glob("*.tmdl"))
        measure_count = 0
        unformatted_measures: list[str] = []

        for tmdl_file in tmdl_files:
            lines = tmdl_file.read_text(encoding="utf-8").splitlines()
            object_starts = [
                index
                for index, line in enumerate(lines)
                if re.match(
                    r"^\t(?:measure|column|partition|hierarchy|calculationGroup|annotation)\b",
                    line,
                )
            ]

            for index, line in enumerate(lines):
                if not line.startswith("\tmeasure "):
                    continue

                measure_name = line.removeprefix("\tmeasure ").split(" =", 1)[0].strip("'")
                measure_count += 1
                next_object = next(
                    (object_index for object_index in object_starts if object_index > index),
                    len(lines),
                )
                block = "\n".join(lines[index:next_object])

                if "formatString:" not in block and "formatStringDefinition" not in block:
                    unformatted_measures.append(f"{tmdl_file.name}:[{measure_name}]")
                self.assertTrue(
                    index > 0 and lines[index - 1].startswith("\t/// "),
                    f"Measure [{measure_name}] in {tmdl_file.name} is missing a supported description",
                )

        self.assertEqual(measure_count, 43, "Expected exactly 43 explicit DAX measures across model")
        self.assertEqual(
            unformatted_measures,
            ["Fact_ATOBenchmark.tmdl:[ATO Compliance Risk Profile]"],
            "Only the text-valued risk-profile measure may omit a numeric format string",
        )

    def test_calculation_groups_have_precedence_and_ordinals(self) -> None:
        """Calculation groups must declare explicit precedence and ordinals on calculation items."""
        cg_files = [f for f in TMDL_TABLES_DIR.glob("*.tmdl") if "CalcGroup" in f.name]
        self.assertGreaterEqual(len(cg_files), 2, "Expected at least 2 calculation groups")

        for cg_file in cg_files:
            content = cg_file.read_text(encoding="utf-8")
            self.assertIn("calculationGroup", content, f"{cg_file.name} must declare calculationGroup block")
            self.assertIn("precedence:", content, f"{cg_file.name} must specify precedence")
            self.assertIn("ordinal:", content, f"{cg_file.name} calculation items must have ordinals")


if __name__ == "__main__":
    unittest.main()
