"""Mutation differential: the Python static suites against their Rust ports.

    python static_differential.py --python-tests <tests directory from the Python base> \
        --python-generator <tools/generate_fixtures.py from the Python base> [--cases N] [--seed S]

Each case copies the application and the shared trial-balance contract into a
scratch tree, applies one or two seeded mutations to the committed artefacts
(TMDL, PBIR JSON, fixtures, documentation, BPA rules, contract fixtures), then
runs the seven Python suites and the matching Rust test binaries against that
tree. Besides random damage, a third of the mutations rewrite a value in a form
Python reads the same way: decimals in another constructor spelling, dates in
compact or ISO week form, JSON integers as floats, a leading blank CSV line, and
BPA expressions extended with an undefined property that short-circuiting skips.
It reports every case where a test passes in one implementation and fails in the
other.
"""

import argparse
import datetime
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

APP = Path(__file__).resolve().parents[2]
REPO = APP.parents[1]
APP_PATH = APP.relative_to(REPO)
CONTRACT = Path("contracts/xero-trial-balance-v1")
MODULES = {
    "test_tmdl_integrity": "tmdl_integrity",
    "test_pbip_structure": "pbip_structure",
    "test_powerquery_m": "powerquery_m",
    "test_model_bpa_rules": "model_bpa_rules",
    "test_xero_trial_balance_contract": "xero_trial_balance_contract",
    "test_fixtures_balance": "fixtures_balance",
    "test_payday_super_rules": "payday_super_rules",
}
RENAMED = {"all_four_pages_and_review_visuals_are_materialised": "all_pages_and_review_visuals_are_materialised"}
RUST_ONLY = {"evaluator_follows_the_rule_grammar", "decimal_grammar_matches_python_conversion"}
SWAPS = [
    ("KEEPFILTERS(", "FILTER("), ("TREATAS(", "TREAT("), ("OneDirection", "BothDirections"),
    ("crossFilteringBehavior", "crossFilter"), ("formatString:", "formatstring:"), ("\t/// ", "\t// "),
    ("precedence: ", "precedence: -"), ("ordinal:", "ord:"), ("BLANK()", "0"), ("Currency.Type", "Int64.Type"),
    ("let\n", "lett\n"), ("in\n", "inn\n"), (",\n", "\n"), ("Table.SelectRowsWithErrors(", "Table.SelectRows("),
    ("compatibilityLevel: ", "compatibilityLevel: 1"), ('"890"', '"891"'), ("||--o{", "}o--||"),
    ("fromColumn:", "toColumn:"), ("\tmeasure ", "\tmeasure X"), ("isHidden", "isVisible"),
    ("Number.Mod", "Number.Div"), ("Columns=", "Columns=1"), ("090", "90"), ('"12D"', '"13D"'),
    ("altText", "alt"), ("SampleFolder & ", "Folder & "), ("ref table ", "ref tabel "),
    ("ref expression ", "ref expr "), ("Source = ", "Src = "), ("Dim_Account[", "Dim_Accounts["),
    ("DATESBETWEEN(", "DATESINPERIOD("), ("Text.Remove(", "Text.Trim("), ("[Net Profit Before Tax]", "[Net Profit]"),
    ("discourageImplicitMeasures", "discourage"), ("\tpartition ", "\tpartitions "), ("\tcolumn ", "\tcolumn X"),
    ("```", "``"), ("expression ", "expressions "), ("Error.Record(", "Error.Rec("), ("Weights = {", "Weights = {1, "),
    ("relationship '", "relationship ''"), ('"Literal"', '"literal"'), ("cardVisual", "card"),
    ("slicer", "slicers"), ("pageNavigator", "navigator"), ("Fx_ValidateABN(", "Fx_Validate("),
    ("Table.Group(", "Table.Combine("), ("&&", "||"), ("<>", "=="), ("<=", "<"), ("!", ""),
    ("IsNullOrEmpty", "IsNull"), ("Precedence", "Order"), ("\"Scope\"", "\"Scopes\""), ("ANZSIC_Code", "ANZSIC"),
    ("consolidat", "Consolidat"), ("Revenue", "Income"), ("Cash", "Money"), ("Working Capital", "Capital"),
    ("mermaid", "mermaid2"), ("erDiagram", "classDiagram"), ("verify.yml", "verify.yaml"),
    ("standard-library-components.yml", "components.yml"), ("tree/main", "tree/dev"),
]
RUNNER = r"""
import json, sys, unittest
results = {}
class Result(unittest.TestResult):
    def _mark(self, test, ok):
        name = test.id().rsplit(".", 1)[-1]
        results[name] = results.get(name, True) and ok
    def addSuccess(self, test): self._mark(test, True)
    def addFailure(self, test, err): self._mark(test, False)
    def addError(self, test, err):
        self._mark(getattr(test, "test_case", test), False)
    def addSkip(self, test, reason): results.setdefault(test.id().rsplit(".", 1)[-1], None)
    def addExpectedFailure(self, test, err): self._mark(test, True)
    def addUnexpectedSuccess(self, test): self._mark(test, False)
    def addSubTest(self, test, subtest, err):
        if err is not None: self._mark(test, False)
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
suite.run(Result())
print(json.dumps(results))
"""


def artefacts(root: Path) -> list[Path]:
    app = root / APP_PATH
    found = [path for path in sorted(app.rglob("*")) if path.is_file() and (
        path.suffix in (".tmdl", ".json", ".pbir", ".pbism", ".pbip", ".csv") or path.name == ".platform")]
    found = [path for path in found if "node_modules" not in path.parts and path.name != "package-lock.json"
             and path.name != "package.json" and "target" not in path.parts and "differential" not in path.parts]
    found += [app / "README.md", app / "docs" / "data-model.md"]
    found += sorted((root / CONTRACT / "fixtures").glob("*.csv"))
    return found


def mutate_json(text: str, rng: random.Random) -> str:
    document = json.loads(text)
    leaves: list[tuple[object, object]] = []

    def walk(node: object) -> None:
        items = node.items() if isinstance(node, dict) else enumerate(node) if isinstance(node, list) else ()
        for key, value in items:
            leaves.append((node, key))
            walk(value)

    walk(document)
    if not leaves:
        return text
    parent, key = rng.choice(leaves)
    value = parent[key]  # type: ignore[index]
    choice = rng.randrange(4)
    if choice == 0 and isinstance(parent, dict):
        del parent[key]
    elif isinstance(value, bool):
        parent[key] = not value  # type: ignore[index]
    elif isinstance(value, (int, float)):
        parent[key] = value + rng.choice((-8, -4, -1, 1, 4, 8, 300))  # type: ignore[index]
    elif isinstance(value, str):
        parent[key] = rng.choice(("", value + "x", value[:-1], value.upper(), "'" + value + "'"))  # type: ignore[index]
    elif isinstance(value, list) and value:
        value.pop(rng.randrange(len(value)))
    elif isinstance(value, dict) and value:
        del value[rng.choice(list(value))]
    else:
        parent[key] = None  # type: ignore[index]
    return json.dumps(document, indent=2, ensure_ascii=False) + "\n"


def mutate_text(text: str, rng: random.Random) -> str:
    lines = text.split("\n")
    operator = rng.randrange(6)
    if operator == 0 and len(lines) > 1:
        del lines[rng.randrange(len(lines))]
    elif operator == 1:
        index = rng.randrange(len(lines))
        lines.insert(index, lines[index])
    elif operator == 2 and len(lines) > 2:
        index = rng.randrange(len(lines) - 1)
        lines[index], lines[index + 1] = lines[index + 1], lines[index]
    elif operator == 3:
        digits = [m.start() for m in re.finditer(r"\d", text)]
        if digits:
            at = rng.choice(digits)
            return text[:at] + str((int(text[at]) + rng.randint(1, 9)) % 10) + text[at + 1:]
    elif operator == 4 and text:
        at = rng.randrange(len(text))
        return text[:at] + text[at + 1:]
    else:
        present = [(old, new) for old, new in SWAPS if old in text]
        if present:
            old, new = rng.choice(present)
            spots = [m.start() for m in re.finditer(re.escape(old), text)]
            at = rng.choice(spots)
            return text[:at] + new + text[at + len(old):]
    return "\n".join(lines)


def respell_decimal(text: str, rng: random.Random) -> str:
    """One two-place amount written another way Decimal() reads as the same value."""
    spots = list(re.finditer(r"(?<![\w.])(-?)(\d+)\.(\d\d)(?![\w.])", text))
    if not spots:
        return text
    spot = rng.choice(spots)
    sign, whole, cents = spot.groups()
    digits = (whole + cents).lstrip("0") or "0"
    form = rng.choice([f"{sign}{digits}E-2", f"{sign or '+'}{whole}.{cents}", f"{sign}{whole[:1]}_{whole[1:]}.{cents}",
                       f" {sign}{whole}.{cents} ", f"{sign}{whole}.{cents}000"])
    return text[:spot.start()] + form + text[spot.end():]


def respell_date(text: str, rng: random.Random) -> str:
    """One ISO date written in compact or ISO week form."""
    spots = list(re.finditer(r"(?<!\d)(\d{4})-(\d\d)-(\d\d)(?!\d)", text))
    if not spots:
        return text
    spot = rng.choice(spots)
    day = datetime.date(*map(int, spot.groups()))
    year, week, weekday = day.isocalendar()
    form = rng.choice([day.strftime("%Y%m%d"), f"{year:04d}-W{week:02d}-{weekday}", f"{year:04d}W{week:02d}{weekday}"])
    return text[:spot.start()] + form + text[spot.end():]


def floats_for_ints(text: str, rng: random.Random) -> str:
    """One JSON integer rewritten as the equal float."""
    document = json.loads(text)
    places: list[tuple[Any, Any]] = []

    def walk(node: Any) -> None:
        items = node.items() if isinstance(node, dict) else enumerate(node) if isinstance(node, list) else ()
        for key, value in items:
            if isinstance(value, int) and not isinstance(value, bool):
                places.append((node, key))
            walk(value)

    walk(document)
    if not places:
        return text
    parent, key = rng.choice(places)
    parent[key] = float(parent[key])
    return json.dumps(document, indent=2, ensure_ascii=False) + "\n"


def skipped_property(text: str, rng: random.Random) -> str:
    """A BPA expression extended so an undefined property sits on a skipped branch."""
    rules = json.loads(text)
    rule = rng.choice(rules)
    rule["Expression"] = rng.choice([f"({rule['Expression']}) && Name == \"\" && string.IsNullOrEmpty(Unknown)",
                                     f"Name <> \"\" || Unknown == 1 || ({rule['Expression']})",
                                     f"({rule['Expression']}) || Unknown"])
    return json.dumps(rules, indent=2) + "\n"


def equivalent(path: Path, text: str, rng: random.Random) -> tuple[str, str]:
    """A rewrite Python reads as the same value, where the file has one."""
    if path.name == "model_bpa_rules.json":
        return skipped_property(text, rng), "skipped property"
    if path.suffix == ".csv":
        choice = rng.randrange(3)
        if choice == 0:
            return "\n" + text, "leading blank line"
        if choice == 1:
            return respell_decimal(text, rng), "decimal spelling"
        return respell_date(text, rng), "date spelling"
    if path.suffix in (".json", ".pbir", ".pbism", ".pbip") or path.name == ".platform":
        try:
            return floats_for_ints(text, rng), "int as float"
        except ValueError:
            pass
    return mutate_text(text, rng), "text"


def mutate(root: Path, rng: random.Random) -> str:
    path = rng.choice(artefacts(root))
    label = path.relative_to(root).as_posix()
    if rng.random() < 0.02:
        path.unlink()
        return f"{label}: deleted"
    text = path.read_bytes().decode("utf-8")
    structured = path.suffix in (".json", ".pbir", ".pbism", ".pbip") or path.name == ".platform"
    if rng.random() < 1 / 3:
        changed, how = equivalent(path, text, rng)
    elif structured and rng.random() < 0.6:
        try:
            changed, how = mutate_json(text, rng), "json value"
        except ValueError:
            changed, how = mutate_text(text, rng), "text"
    else:
        changed, how = mutate_text(text, rng), "text"
    path.write_bytes(changed.encode("utf-8"))
    return f"{label}: {how}"


def build_rust() -> dict[str, str]:
    output = subprocess.run(["cargo", "test", "--locked", "--no-run", "--message-format=json"], cwd=APP,
                            capture_output=True, text=True, check=True).stdout
    binaries = {}
    for line in output.splitlines():
        message = json.loads(line)
        target = message.get("target", {})
        if message.get("reason") == "compiler-artifact" and message.get("executable") and target.get("name") in MODULES.values():
            binaries[target["name"]] = message["executable"]
    if set(binaries) != set(MODULES.values()):
        raise SystemExit("missing Rust test binaries: " + ", ".join(sorted(set(MODULES.values()) - set(binaries))))
    return binaries


def run_rust(binaries: dict[str, str], app: Path) -> dict[str, bool]:
    results: dict[str, bool] = {}
    environment = dict(os.environ, POWERBI_APP_DIR=str(app), RUST_BACKTRACE="0")
    for binary in binaries.values():
        completed = subprocess.run([binary, "--test-threads=4"], capture_output=True, text=True, env=environment, timeout=120)
        for match in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED)$", completed.stdout, re.MULTILINE):
            results[match.group(1)] = match.group(2) == "ok"
    return {name: ok for name, ok in results.items() if name not in RUST_ONLY}


def run_python(tests: Path) -> dict[str, bool]:
    completed = subprocess.run([sys.executable, "-B", "-c", RUNNER, *MODULES], cwd=tests, capture_output=True,
                               text=True, timeout=300)
    if completed.returncode:
        raise SystemExit("Python runner failed: " + completed.stderr[-2000:])
    raw: dict[str, bool | None] = json.loads(completed.stdout)
    return {RENAMED.get(name[5:], name[5:]): ok for name, ok in raw.items() if ok is not None}


def prepare(scratch: Path, python_tests: Path, generator: Path) -> Path:
    root = scratch / "repo"
    ignore = shutil.ignore_patterns("target", "node_modules", "__pycache__", ".venv")
    shutil.copytree(APP, root / APP_PATH, ignore=ignore)
    shutil.copytree(REPO / CONTRACT, root / CONTRACT)
    for module in MODULES:
        shutil.copy2(python_tests / f"{module}.py", root / APP_PATH / "tests" / f"{module}.py")
    shutil.copy2(generator, root / APP_PATH / "tools" / "generate_fixtures.py")
    return root


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-tests", type=Path, required=True)
    parser.add_argument("--python-generator", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=200)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    binaries = build_rust()
    differences = failing = 0
    with tempfile.TemporaryDirectory(prefix="pbi-static-") as scratch:
        pristine = prepare(Path(scratch), args.python_tests.resolve(), args.python_generator.resolve())
        baseline_python = run_python(pristine / APP_PATH / "tests")
        baseline_rust = run_rust(binaries, pristine / APP_PATH)
        if set(baseline_python) != set(baseline_rust) or not all(baseline_python.values()) or not all(baseline_rust.values()):
            print("baseline mismatch", sorted(set(baseline_python) ^ set(baseline_rust)),
                  [n for n, ok in baseline_python.items() if not ok], [n for n, ok in baseline_rust.items() if not ok])
            return 1
        for case in range(args.cases):
            rng = random.Random(f"{args.seed}:{case}")
            root = Path(scratch) / f"case-{case}"
            shutil.copytree(pristine, root)
            labels = [mutate(root, rng) for _ in range(rng.choice((1, 1, 2)))]
            python = run_python(root / APP_PATH / "tests")
            rust = run_rust(binaries, root / APP_PATH)
            differing = sorted(name for name in baseline_python if python.get(name) != rust.get(name))
            failing += not all(python.values())
            if differing:
                differences += 1
                print(f"case {case}: {'; '.join(labels)}")
                for name in differing:
                    print(f"  {name}: python={python.get(name)} rust={rust.get(name)}")
            shutil.rmtree(root)
    print(f"{args.cases} cases, {failing} failing at least one test, {differences} with differences, "
          f"{len(baseline_python)} tests compared")
    return 1 if differences else 0


if __name__ == "__main__":
    sys.exit(main())
