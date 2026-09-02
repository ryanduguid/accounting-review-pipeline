"""Contracts for a loadable, source-controlled Power BI project."""

from __future__ import annotations

import json
import re
import unittest
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
NAME = "australian-accounting-power-bi"
MODEL = ROOT / f"{NAME}.SemanticModel"
DEFINITION = MODEL / "definition"
REPORT = ROOT / f"{NAME}.Report"
REPORT_DEFINITION = REPORT / "definition"


def read_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8"))


def tmdl_field_inventory() -> dict[str, dict[str, set[str]]]:
    """Inventory columns and measures declared by each TMDL table."""
    inventory: dict[str, dict[str, set[str]]] = {}

    for path in sorted((DEFINITION / "tables").glob("*.tmdl")):
        fields = {"Column": set(), "Measure": set()}
        for line in path.read_text(encoding="utf-8").splitlines():
            for kind in fields:
                prefix = f"\t{kind.lower()} "
                if not line.startswith(prefix):
                    continue
                raw_name = line.removeprefix(prefix)
                if kind == "Measure":
                    raw_name = raw_name.split(" =", 1)[0]
                fields[kind].add(raw_name.strip().strip("'").replace("''", "'"))
        inventory[path.stem] = fields

    return inventory


# Metric words that a visual title may only use when the visual actually binds a field
# whose table or name carries the same word. Deliberately excludes words a title can
# legitimately use without a matching binding, such as "Intercompany" on the rollup
# pivot, where the elimination is applied by CalcGroup_Consolidation.
TITLE_METRIC_WORDS = (
    "Cash",
    "Revenue",
    "EBITDA",
    "Working Capital",
    "Net Assets",
    "GIC",
    "SGC",
    "Benchmark",
    "Compliance",
    "Expense",
)


def visual_title(visual: object) -> str:
    """Return a visual's rendered title literal, or an empty string when it has none."""
    if not isinstance(visual, dict):
        return ""
    titles = visual.get("visualContainerObjects", {}).get("title", [])
    if not isinstance(titles, list) or not titles:
        return ""
    literal = (
        titles[0]
        .get("properties", {})
        .get("text", {})
        .get("expr", {})
        .get("Literal", {})
        .get("Value", "")
    )
    return literal.strip("'") if isinstance(literal, str) else ""


def query_field_bindings(node: object) -> list[tuple[str, str, str]]:
    """Recursively collect (kind, table, field) bindings from PBIR query state."""
    bindings: list[tuple[str, str, str]] = []

    if isinstance(node, dict):
        for kind in ("Column", "Measure"):
            binding = node.get(kind)
            if isinstance(binding, dict):
                expression = binding.get("Expression")
                source_ref = expression.get("SourceRef") if isinstance(expression, dict) else None
                entity = source_ref.get("Entity") if isinstance(source_ref, dict) else None
                field = binding.get("Property")
                if isinstance(entity, str) and isinstance(field, str):
                    bindings.append((kind, entity, field))
        for value in node.values():
            bindings.extend(query_field_bindings(value))
    elif isinstance(node, list):
        for value in node:
            bindings.extend(query_field_bindings(value))

    return bindings


class SemanticModelStructureTests(unittest.TestCase):
    def test_semantic_model_uses_the_supported_tmdl_folder_layout(self) -> None:
        descriptor = read_json(MODEL / "definition.pbism")

        self.assertEqual(descriptor["version"], "4.0")
        self.assertIn("semanticModel/definitionProperties/1.0.0", descriptor["$schema"])
        self.assertTrue((DEFINITION / "database.tmdl").is_file())
        self.assertTrue((DEFINITION / "model.tmdl").is_file())
        self.assertTrue((DEFINITION / "relationships.tmdl").is_file())
        self.assertTrue((DEFINITION / "expressions.tmdl").is_file())
        self.assertEqual(sorted(path.name for path in MODEL.glob("*.tmdl")), [])

    def test_model_references_every_table_and_named_expression(self) -> None:
        model = (DEFINITION / "model.tmdl").read_text(encoding="utf-8")
        table_names = sorted(path.stem for path in (DEFINITION / "tables").glob("*.tmdl"))
        expression_text = (DEFINITION / "expressions.tmdl").read_text(encoding="utf-8")
        expression_names = sorted(
            match.strip("'")
            for match in re.findall(r"^expression\s+('[^']+'|[^\s=]+)\s*=", expression_text, re.MULTILINE)
        )

        self.assertEqual(len(table_names), 11)
        self.assertEqual(
            expression_names,
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
        for name in table_names:
            self.assertIn(f"ref table {name}", model)
        for name in expression_names:
            self.assertIn(f"ref expression {name}", model)

    def test_every_partition_symbol_resolves_to_a_named_expression(self) -> None:
        expression_text = (DEFINITION / "expressions.tmdl").read_text(encoding="utf-8")
        expression_names = {
            match.strip("'")
            for match in re.findall(r"^expression\s+('[^']+'|[^\s=]+)\s*=", expression_text, re.MULTILINE)
        }
        standard_library_roots = {"Table"}
        unresolved: list[str] = []
        partition_count = 0

        for path in sorted((DEFINITION / "tables").glob("*.tmdl")):
            content = path.read_text(encoding="utf-8")
            partition_count += len(re.findall(r"^\tpartition\s", content, re.MULTILINE))
            for symbol in re.findall(r"^\t\t\t\tSource = ([A-Za-z_][A-Za-z0-9_]*)", content, re.MULTILINE):
                if symbol not in expression_names and symbol not in standard_library_roots:
                    unresolved.append(f"{path.name}:{symbol}")

        # Nine import partitions are declared here. The two calculation-group
        # tables receive implicit partitions when the model is loaded.
        self.assertEqual(partition_count, 9)
        self.assertEqual(unresolved, [])

    def test_measure_descriptions_use_supported_triple_slash_syntax(self) -> None:
        missing: list[str] = []
        unsupported: list[str] = []
        measure_count = 0

        for path in sorted((DEFINITION / "tables").glob("*.tmdl")):
            lines = path.read_text(encoding="utf-8").splitlines()
            for index, line in enumerate(lines):
                if line.startswith("\t\tdescription:"):
                    unsupported.append(f"{path.name}:{index + 1}")
                if line.startswith("\tmeasure "):
                    measure_count += 1
                    if index == 0 or not lines[index - 1].startswith("\t/// "):
                        missing.append(f"{path.name}:{index + 1}")

        self.assertEqual(measure_count, 43)
        self.assertEqual(unsupported, [])
        self.assertEqual(missing, [])

    def test_power_query_has_one_canonical_tmdl_source(self) -> None:
        self.assertFalse((ROOT / "powerquery").exists())
        expressions = (DEFINITION / "expressions.tmdl").read_text(encoding="utf-8")
        self.assertIn('File.Contents("samples/sample-budgets.csv")', expressions)


class ReportStructureTests(unittest.TestCase):
    def test_report_scaffold_uses_published_pbir_contracts(self) -> None:
        platform = read_json(REPORT / ".platform")
        binding = read_json(REPORT / "definition.pbir")
        version = read_json(REPORT_DEFINITION / "version.json")
        report = read_json(REPORT_DEFINITION / "report.json")
        pages = read_json(REPORT_DEFINITION / "pages" / "pages.json")

        self.assertEqual(platform["metadata"]["type"], "Report")
        self.assertEqual(binding["version"], "4.0")
        self.assertIn("report/definitionProperties/1.0.0", binding["$schema"])
        self.assertEqual(version["version"], "2.0.0")
        self.assertIn("report/", report["$schema"])
        self.assertEqual(len(pages["pageOrder"]), 4)
        self.assertEqual(pages["activePageName"], pages["pageOrder"][0])

    def test_all_four_pages_and_twenty_one_visuals_are_materialised(self) -> None:
        pages = read_json(REPORT_DEFINITION / "pages" / "pages.json")
        visual_types: Counter[str] = Counter()
        unbound: list[str] = []

        for page_name in pages["pageOrder"]:
            page_dir = REPORT_DEFINITION / "pages" / page_name
            page = read_json(page_dir / "page.json")
            self.assertEqual(page["name"], page_name)
            self.assertEqual((page["width"], page["height"]), (1280, 720))
            for visual_path in sorted((page_dir / "visuals").glob("*/visual.json")):
                document = read_json(visual_path)
                self.assertEqual(document["name"], visual_path.parent.name)
                visual = document["visual"]
                visual_type = visual["visualType"]
                visual_types[visual_type] += 1
                if visual_type != "textbox" and not visual.get("query", {}).get("queryState"):
                    unbound.append(str(visual_path.relative_to(ROOT)))

        self.assertEqual(
            visual_types,
            Counter(
                {
                    "cardVisual": 10,
                    "textbox": 4,
                    "pivotTable": 3,
                    "tableEx": 2,
                    "lineChart": 1,
                    "scatterChart": 1,
                }
            ),
        )
        self.assertEqual(sum(visual_types.values()), 21)
        self.assertEqual(unbound, [])

    def test_every_visual_field_binding_resolves_to_the_semantic_model(self) -> None:
        inventory = tmdl_field_inventory()
        bindings: list[tuple[str, str, str, str]] = []

        for visual_path in sorted(
            (REPORT_DEFINITION / "pages").glob("*/visuals/*/visual.json")
        ):
            document = read_json(visual_path)
            visual = document.get("visual")
            query = visual.get("query") if isinstance(visual, dict) else None
            query_state = query.get("queryState") if isinstance(query, dict) else None
            if query_state is None:
                continue
            relative_path = str(visual_path.relative_to(ROOT))
            bindings.extend(
                (kind, table, field, relative_path)
                for kind, table, field in query_field_bindings(query_state)
            )

        missing = [
            f"{path}: {kind} {table}[{field}]"
            for kind, table, field, path in bindings
            if table not in inventory or field not in inventory[table][kind]
        ]

        self.assertGreater(len(bindings), 0, "Expected data-bound PBIR visuals")
        self.assertEqual(missing, [])

    def test_visual_titles_do_not_name_metrics_the_visual_does_not_plot(self) -> None:
        """A title is the only label a reader gets, so it must not name an absent metric.

        The line chart on page 1 was titled "Working Capital & Operating Cash Trend" while
        plotting Working Capital and Net Assets. The model has no cash measure at all, so
        the title promised a series that could not be there and that no reader could catch.
        """
        titled = 0
        unsupported: list[str] = []

        for visual_path in sorted(
            (REPORT_DEFINITION / "pages").glob("*/visuals/*/visual.json")
        ):
            visual = read_json(visual_path).get("visual")
            title = visual_title(visual)
            if not title:
                continue
            titled += 1
            query = visual.get("query") if isinstance(visual, dict) else None
            query_state = query.get("queryState") if isinstance(query, dict) else None
            bound = " ".join(
                f"{table} {field}"
                for _, table, field in query_field_bindings(query_state or {})
            ).lower()
            unsupported.extend(
                f"{visual_path.parent.name}: title says {word!r}, no field binding does"
                for word in TITLE_METRIC_WORDS
                if word.lower() in title.lower() and word.lower() not in bound
            )

        self.assertEqual(titled, 17, "Expected 17 titled visuals across the four pages")
        self.assertEqual(unsupported, [])


if __name__ == "__main__":
    unittest.main()
