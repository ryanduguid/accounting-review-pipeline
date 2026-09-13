"""
test_model_bpa_rules.py - Asserts each Best Practice Analyzer rule selects violations, not compliant objects.

A BPA rule expression selects the objects that break the rule, so a predicate written the
right way round is false for the shipped model. The format and description rules were
written as `!string.IsNullOrEmpty(...)`, which selected the 45 formatted and 47 described
measures and reported every compliant measure as a violation. Tabular Editor is not run
here, so this suite evaluates the committed expressions against metadata parsed from TMDL.
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
RULES_FILE = BASE_DIR / "tests" / "model_bpa_rules.json"
TMDL_DIR = BASE_DIR / "australian-accounting-power-bi.SemanticModel" / "definition"
TMDL_TABLES_DIR = TMDL_DIR / "tables"

# TMDL declares no dataType on a measure; the engine infers it from the expression. These
# two return text, which is why the format rule exempts them and why the TMDL integrity
# suite allows them to ship without a format string.
TEXT_MEASURES = {"ATO Compliance Risk Profile", "Benchmark Turnover Band"}


def evaluate(expression: str, obj: dict[str, object]) -> bool:
    """Evaluate one Dynamic LINQ rule expression against a single object's properties."""
    translated = re.sub(
        r"string\.IsNullOrEmpty\((\w+)\)", r"(not \1)", expression, flags=re.IGNORECASE
    )
    translated = translated.replace("&&", " and ").replace("||", " or ")
    translated = translated.replace("<>", "!=")
    translated = re.sub(r"!(?!=)", " not ", translated)
    return bool(eval(translated, {"__builtins__": {}}, dict(obj)))


def shipped_measures() -> list[dict[str, object]]:
    measures: list[dict[str, object]] = []
    for tmdl_file in sorted(TMDL_TABLES_DIR.glob("*.tmdl")):
        lines = tmdl_file.read_text(encoding="utf-8").splitlines()
        starts = [
            index
            for index, line in enumerate(lines)
            if re.match(
                r"^\t(?:measure|column|partition|hierarchy|calculationGroup|annotation)\b", line
            )
        ]
        for index, line in enumerate(lines):
            if not line.startswith("\tmeasure "):
                continue
            name = line.removeprefix("\tmeasure ").split(" =", 1)[0].strip("'")
            end = next((start for start in starts if start > index), len(lines))
            block = "\n".join(lines[index:end])
            format_string = re.search(r"^\t\tformatString: (.+)$", block, re.MULTILINE)
            previous = lines[index - 1] if index else ""
            measures.append(
                {
                    "Name": name,
                    "FormatString": format_string.group(1) if format_string else "",
                    "Description": (
                        previous.removeprefix("\t/// ") if previous.startswith("\t/// ") else ""
                    ),
                    "IsHidden": bool(re.search(r"^\t\tisHidden", block, re.MULTILINE)),
                    "DataType": "String" if name in TEXT_MEASURES else "Decimal",
                }
            )
    return measures


def shipped_relationships() -> list[dict[str, object]]:
    text = (TMDL_DIR / "relationships.tmdl").read_text(encoding="utf-8")
    names = re.findall(r"^relationship '([^']+)'", text, re.MULTILINE)
    blocks = re.split(r"^relationship '[^']+'", text, flags=re.MULTILINE)[1:]
    relationships: list[dict[str, object]] = []
    for name, block in zip(names, blocks):
        # TMDL omits crossFilteringBehavior when the relationship filters one way.
        behaviour = re.search(r"crossFilteringBehavior: (\w+)", block)
        relationships.append(
            {
                "Name": name,
                "CrossFilteringBehavior": behaviour.group(1) if behaviour else "OneDirection",
            }
        )
    return relationships


def shipped_calculation_groups() -> list[dict[str, object]]:
    groups: list[dict[str, object]] = []
    for tmdl_file in sorted(TMDL_TABLES_DIR.glob("*.tmdl")):
        text = tmdl_file.read_text(encoding="utf-8")
        if "calculationGroup" not in text:
            continue
        precedence = re.search(r"precedence: (-?\d+)", text)
        groups.append(
            {
                "Name": tmdl_file.stem,
                "Precedence": int(precedence.group(1)) if precedence else 0,
            }
        )
    return groups


class TestModelBpaRules(unittest.TestCase):
    def setUp(self) -> None:
        self.rules = {rule["ID"]: rule for rule in json.loads(RULES_FILE.read_text(encoding="utf-8"))}

    def test_every_rule_declares_the_fields_the_analyzer_reads(self) -> None:
        for rule_id, rule in self.rules.items():
            for field in ("Name", "Category", "Severity", "Scope", "Expression"):
                self.assertIn(field, rule, f"{rule_id} is missing {field}")

    def test_no_rule_selects_a_compliant_shipped_object(self) -> None:
        """The shipped model satisfies all 4 rules, so a correct expression selects nothing."""
        scopes = {
            "Measure": shipped_measures(),
            "Relationship": shipped_relationships(),
            "CalculationGroup": shipped_calculation_groups(),
        }
        for name, objects in scopes.items():
            self.assertTrue(objects, f"No {name} metadata was parsed out of the TMDL")

        for rule_id, rule in self.rules.items():
            selected = [
                obj["Name"]
                for obj in scopes[rule["Scope"]]
                if evaluate(rule["Expression"], obj)
            ]
            self.assertEqual(
                selected,
                [],
                f"{rule_id} selects compliant objects; a BPA expression selects violations",
            )

    def test_each_rule_selects_its_own_violation_and_spares_the_control(self) -> None:
        """Each rule must flag a crafted violator while leaving a nearby valid object alone."""
        cases: dict[str, tuple[dict[str, object], dict[str, object]]] = {
            "BPA_MEASURE_FORMAT_STRING": (
                {"Name": "violator", "FormatString": "", "DataType": "Decimal"},
                {"Name": "control", "FormatString": "$#,##0.00", "DataType": "Decimal"},
            ),
            "BPA_MEASURE_DESCRIPTION": (
                {"Name": "violator", "IsHidden": False, "Description": ""},
                {"Name": "control", "IsHidden": False, "Description": "Documented."},
            ),
            "BPA_STAR_SCHEMA_RELATIONSHIPS": (
                {"Name": "violator", "CrossFilteringBehavior": "BothDirections"},
                {"Name": "control", "CrossFilteringBehavior": "OneDirection"},
            ),
            "BPA_CALC_GROUP_PRECEDENCE": (
                {"Name": "violator", "Precedence": 0},
                {"Name": "control", "Precedence": 10},
            ),
        }
        self.assertEqual(sorted(cases), sorted(self.rules), "Every rule needs a violation case")

        for rule_id, (violator, control) in cases.items():
            expression = self.rules[rule_id]["Expression"]
            self.assertTrue(
                evaluate(expression, violator), f"{rule_id} must select {violator}"
            )
            self.assertFalse(
                evaluate(expression, control), f"{rule_id} must not select {control}"
            )

    def test_the_format_rule_exempts_a_text_measure(self) -> None:
        """A measure that returns text has no numeric format to require."""
        expression = self.rules["BPA_MEASURE_FORMAT_STRING"]["Expression"]
        self.assertFalse(
            evaluate(expression, {"Name": "status", "FormatString": "", "DataType": "String"})
        )
        self.assertTrue(
            evaluate(expression, {"Name": "amount", "FormatString": "", "DataType": "Decimal"})
        )


if __name__ == "__main__":
    unittest.main()
