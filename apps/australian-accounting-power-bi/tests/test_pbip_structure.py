"""Contracts for a loadable, source-controlled Power BI project."""

from __future__ import annotations

import json
import re
import unittest
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
NAME = "australian-accounting-power-bi"
MODEL = ROOT / f"{NAME}.SemanticModel"
DEFINITION = MODEL / "definition"
REPORT = ROOT / f"{NAME}.Report"
REPORT_DEFINITION = REPORT / "definition"


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def tmdl_field_inventory() -> dict[str, dict[str, set[str]]]:
    """Inventory columns and measures declared by each TMDL table."""
    inventory: dict[str, dict[str, set[str]]] = {}

    for path in sorted((DEFINITION / "tables").glob("*.tmdl")):
        fields: dict[str, set[str]] = {"Column": set(), "Measure": set()}
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

        self.assertEqual(len(table_names), 14)
        self.assertEqual(
            sorted(set(table_names) & set(expression_names)),
            [],
            "Power Query table queries and named expressions must have distinct names",
        )
        self.assertEqual(
            expression_names,
            [
                "Dim_Date_AU",
                "Fx_MoneyDisplay",
                "Fx_RequireRows",
                "Fx_ValidateABN",
                "IndustryLookup",
                "ModelEndDate",
                "ModelStartDate",
                "SampleFolder",
                "Source_Dim_Account",
                "Source_Dim_Entity",
                "Source_Fact_ATOBenchmark",
                "Source_Fact_Budget",
                "Source_Fact_GeneralLedger",
                "Source_Fact_PayrollSuper",
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

        # Nine import partitions are declared here. The 2 calculation-group
        # tables receive implicit partitions when the model is loaded.
        self.assertEqual(partition_count, 12)
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

        self.assertEqual(measure_count, 63)
        self.assertEqual(unsupported, [])
        self.assertEqual(missing, [])

    def test_power_query_has_one_canonical_tmdl_source(self) -> None:
        self.assertFalse((ROOT / "powerquery").exists())
        expressions = (DEFINITION / "expressions.tmdl").read_text(encoding="utf-8")
        sources = [expressions]
        sources.extend(
            path.read_text(encoding="utf-8")
            for path in (DEFINITION / "tables").glob("*.tmdl")
        )
        file_paths = re.findall(r"File\.Contents\(([^)]+)\)", "\n".join(sources))
        self.assertEqual(len(file_paths), 9)
        for file_path in file_paths:
            self.assertTrue(
                file_path.startswith("SampleFolder & "),
                f"CSV imports need the configured absolute sample folder: {file_path}",
            )


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
        self.assertEqual(len(pages["pageOrder"]), 6)
        self.assertEqual(pages["activePageName"], pages["pageOrder"][0])

    def test_all_four_pages_and_review_visuals_are_materialised(self) -> None:
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
                if visual_type not in {"textbox", "pageNavigator", "actionButton"} and not visual.get("query", {}).get("queryState"):
                    unbound.append(str(visual_path.relative_to(ROOT)))

        self.assertEqual(visual_types["pageNavigator"], len(pages["pageOrder"]) - 1)
        self.assertGreaterEqual(visual_types["slicer"], 2 * (len(pages["pageOrder"]) - 1))
        self.assertGreater(visual_types["lineChart"], 0)
        self.assertEqual(unbound, [])

    def test_pages_have_selectors_alt_text_and_unique_keyboard_order(self) -> None:
        for page_path in REPORT_DEFINITION.glob("pages/*/page.json"):
            visuals = [read_json(path) for path in page_path.parent.glob("visuals/*/visual.json")]
            selectors = [obj for obj in visuals if obj["visual"]["visualType"] == "slicer"]
            bindings = {field for obj in selectors for _, _, field in query_field_bindings(obj)}
            if page_path.parent.name == "a01closehome202410001":
                self.assertTrue({"Entity", "Period"}.issubset(bindings))
            elif page_path.parent.name != "a02closeevid202410002":
                self.assertTrue({"TradingName", "FinancialYear"}.issubset(bindings))
            orders = [obj["position"]["tabOrder"] for obj in visuals]
            self.assertEqual(len(orders), len(set(orders)))
            # Native Desktop visits the higher tabOrder values first.
            keyboard_order = sorted(visuals, key=lambda obj: obj["position"]["tabOrder"], reverse=True)
            reading_order = sorted(visuals, key=lambda obj: (obj["position"]["y"], obj["position"]["x"]))
            self.assertEqual([obj["name"] for obj in keyboard_order], [obj["name"] for obj in reading_order])
            for obj in visuals:
                position = obj["position"]
                page = read_json(page_path)
                self.assertGreaterEqual(position["x"], 0)
                self.assertGreaterEqual(position["y"], 0)
                self.assertLessEqual(position["x"] + position["width"], page["width"])
                self.assertLessEqual(position["y"] + position["height"], page["height"])
                general = obj["visual"]["visualContainerObjects"]["general"][0]
                alt = general["properties"]["altText"]["expr"]["Literal"]["Value"]
                self.assertGreater(len(alt), 15, obj["name"])

    def test_phone_layouts_preserve_reading_order_and_touch_spacing(self) -> None:
        for page_path in REPORT_DEFINITION.glob("pages/*/page.json"):
            visuals = [read_json(path) for path in page_path.parent.glob("visuals/*/visual.json")]
            expected = sorted(visuals, key=lambda obj: (obj["position"]["y"], obj["position"]["x"]))
            mobile = [read_json(page_path.parent / "visuals" / obj["name"] / "mobile.json") for obj in expected]
            last_bottom = -8
            for index, (visual, portrait) in enumerate(zip(expected, mobile)):
                with self.subTest(page=page_path.parent.name, visual=visual["name"]):
                    self.assertIn("visualContainerMobileState/2.4.0/schema.json", portrait["$schema"])
                    position = portrait["position"]
                    self.assertEqual((position["x"], position["width"]), (0, 324))
                    self.assertGreaterEqual(position["y"], last_bottom + 8)
                    self.assertGreaterEqual(position["height"], 56)
                    self.assertEqual(position["tabOrder"], len(expected) - index - 1)
                    self.assertEqual(position["y"] % 4, 0)
                    self.assertEqual(position["height"] % 4, 0)
                    last_bottom = position["y"] + position["height"]
                    if visual["visual"]["visualType"] == "pageNavigator":
                        layout = portrait["objects"]["layout"][0]["properties"]
                        self.assertEqual(layout["rowCount"]["expr"]["Literal"]["Value"], "3D")
                        self.assertEqual(layout["columnCount"]["expr"]["Literal"]["Value"], "2D")
                        self.assertGreaterEqual((position["height"] - 16) / 3, 44)

    def test_phone_selected_finding_details_fit_the_canvas(self) -> None:
        path = REPORT_DEFINITION / "pages/a01closehome202410001/visuals/reviewhomequestions/mobile.json"
        portrait = read_json(path)
        objects = portrait["objects"]
        label = float(objects["columnHeaders"][0]["properties"]["defaultColumnWidth"]["expr"]["Literal"]["Value"][:-1])
        value = float(objects["columnWidth"][0]["properties"]["value"]["expr"]["Literal"]["Value"][:-1])
        self.assertLessEqual(label + value, portrait["position"]["width"] - 16)
        self.assertGreaterEqual(value, 150)

    def test_phone_results_use_the_native_vertical_card_layout(self) -> None:
        for path in REPORT_DEFINITION.glob("pages/*/visuals/*/visual.json"):
            visual = read_json(path)["visual"]
            if visual["visualType"] != "cardVisual":
                continue
            portrait = read_json(path.with_name("mobile.json"))
            layout = portrait["objects"]["layout"][0]
            # Desktop writes card layout properties without a selector.
            # A default selector is silently ignored for this category.
            self.assertNotIn("selector", layout)
            self.assertEqual(layout["properties"]["orientation"]["expr"]["Literal"]["Value"], "1D")
            count = len(visual["query"]["queryState"]["Data"]["projections"])
            self.assertGreaterEqual(portrait["position"]["height"], count * 100)

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
                for kind, table, field in query_field_bindings(document)
            )

        missing = [
            f"{path}: {kind} {table}[{field}]"
            for kind, table, field, path in bindings
            if table not in inventory or field not in inventory[table][kind]
        ]

        self.assertGreater(len(bindings), 0, "Expected data-bound PBIR visuals")
        self.assertEqual(missing, [])

    def test_budget_visuals_use_contextual_title_measures(self) -> None:
        for visual_id, title_measure in (
            ("39ec83f2c78b1ea20481", "Revenue Budget Title"),
            ("2f440022aab813e0dbbb", "Monthly Revenue Title"),
        ):
            path = REPORT_DEFINITION / "pages" / "062c0d9cd2ba960997fe" / "visuals" / visual_id / "visual.json"
            visual = read_json(path)["visual"]
            title = visual["visualContainerObjects"]["title"][0]["properties"]["text"]
            self.assertEqual(query_field_bindings(title), [("Measure", "Fact_GeneralLedger", title_measure)])

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

        self.assertGreater(titled, 20, "Expected titled visuals across all four pages")
        self.assertEqual(unsupported, [])

    def test_only_consolidated_bindings_may_carry_a_consolidated_label(self) -> None:
        """A statement labelled consolidated must apply the consolidation calculation group.

        The executive P&L was titled "Consolidated Statement of Financial Performance (P&L)"
        while binding the ordinary measures, so it displayed $882,000 of intercompany
        revenue: $576,000 of management fees and $306,000 of internal freight. Only
        CalcGroup_Consolidation removes those rows, and it lives on page 2.
        """
        claims: list[str] = []
        checked = 0

        for page_path in sorted((REPORT_DEFINITION / "pages").glob("*/page.json")):
            page_visuals = sorted(page_path.parent.glob("visuals/*/visual.json"))
            consolidated_bindings = {
                table
                for visual_path in page_visuals
                for _, table, _ in query_field_bindings(read_json(visual_path).get("visual"))
                if table == "CalcGroup_Consolidation"
            }

            labels = [(page_path.parent.name, read_json(page_path).get("displayName", ""))]
            labels.extend(
                (visual_path.parent.name, visual_title(read_json(visual_path).get("visual")))
                for visual_path in page_visuals
            )
            for name, label in labels:
                if not isinstance(label, str) or not label:
                    continue
                checked += 1
                if "consolidat" in label.lower() and not consolidated_bindings:
                    claims.append(f"{name}: {label!r} claims consolidation with no binding")

        self.assertGreater(checked, 20, "Expected every page and titled visual to be checked")
        self.assertEqual(claims, [])


class ReportUniformityTests(unittest.TestCase):
    def test_result_cards_keep_shared_labels_surfaces_and_padding(self) -> None:
        visuals = [read_json(path) for path in REPORT_DEFINITION.glob("pages/*/visuals/*/visual.json")]
        cards = [visual for visual in visuals if visual["visual"]["visualType"] == "cardVisual"]
        self.assertEqual(len(cards), 4)
        for card in cards:
            objects = card["visual"]["objects"]
            for role in ("label", "fillCustom", "outline", "padding"):
                self.assertEqual(objects[role], cards[0]["visual"]["objects"][role])
            padding = objects["layout"][0]["properties"]["cellPadding"]
            self.assertEqual(padding["expr"]["Literal"]["Value"], "12D")
            expected = "14D" if card["name"] == "3fd74ef9d4436795bbdd" else "28D"
            font = objects["value"][0]["properties"]["fontSize"]
            self.assertEqual(font["expr"]["Literal"]["Value"], expected)

    def test_query_projection_aliases_are_unique(self) -> None:
        for path in REPORT_DEFINITION.glob("pages/*/visuals/*/visual.json"):
            document = read_json(path)
            aliases = [
                projection["nativeQueryRef"]
                for role in document["visual"].get("query", {}).get("queryState", {}).values()
                for projection in role.get("projections", [])
                if "nativeQueryRef" in projection
            ]
            self.assertEqual(len(aliases), len(set(aliases)), document["name"])

    def test_visuals_follow_the_shared_grid_and_do_not_overlap(self) -> None:
        for page_path in REPORT_DEFINITION.glob("pages/*/page.json"):
            visuals = [read_json(path) for path in page_path.parent.glob("visuals/*/visual.json")]
            for index, visual in enumerate(visuals):
                position = visual["position"]
                for key in ("x", "y", "width", "height"):
                    self.assertEqual(position[key] % 4, 0, (visual["name"], key))
                self.assertGreaterEqual(position["x"], 20)
                self.assertLessEqual(position["x"] + position["width"], 1260)
                self.assertLessEqual(position["y"] + position["height"], 700)
                for other in visuals[index + 1 :]:
                    candidate = other["position"]
                    overlap = (
                        position["x"] < candidate["x"] + candidate["width"]
                        and candidate["x"] < position["x"] + position["width"]
                        and position["y"] < candidate["y"] + candidate["height"]
                        and candidate["y"] < position["y"] + position["height"]
                    )
                    self.assertFalse(overlap, (visual["name"], other["name"]))

    def test_navigation_and_filters_keep_shared_geometry_and_style(self) -> None:
        navigators = []
        for path in REPORT_DEFINITION.glob("pages/*/visuals/*/visual.json"):
            document = read_json(path)
            visual = document["visual"]
            position = document["position"]
            if visual["visualType"] == "pageNavigator":
                navigators.append(document)
                self.assertEqual(
                    [position[key] for key in ("x", "y", "width", "height")],
                    [520, 8, 740, 40],
                )
            elif visual["visualType"] == "slicer":
                self.assertEqual(position["height"], 60, document["name"])
                properties = visual["objects"]["header"][0]["properties"]
                self.assertEqual(properties["show"]["expr"]["Literal"]["Value"], "false")
        self.assertEqual(len(navigators), 5)
        for navigator in navigators[1:]:
            self.assertEqual(navigator["visual"]["objects"], navigators[0]["visual"]["objects"])

    def test_tables_keep_shared_typography_and_row_styles(self) -> None:
        for path in REPORT_DEFINITION.glob("pages/*/visuals/*/visual.json"):
            document = read_json(path)
            visual = document["visual"]
            if visual["visualType"] not in {"tableEx", "pivotTable"}:
                continue
            objects = visual["objects"]
            for group in ("columnHeaders", "values"):
                size = objects[group][0]["properties"]["fontSize"]
                self.assertEqual(size["expr"]["Literal"]["Value"], "12D", document["name"])
            headers = objects["columnHeaders"][0]["properties"]
            self.assertEqual(
                headers["backColor"]["solid"]["color"]["expr"]["Literal"]["Value"],
                "'#E7EDEB'",
            )
            padding = objects["grid"][0]["properties"]["rowPadding"]
            self.assertEqual(padding["expr"]["Literal"]["Value"], "2D")


if __name__ == "__main__":
    unittest.main()
