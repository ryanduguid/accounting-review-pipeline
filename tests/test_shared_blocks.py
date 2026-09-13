"""Pin the code Monthly Close Controls and the Workpaper Review Gate each keep a copy of.

The 2 packages publish independently: separate distributions, versions,
lockfiles, licences and release tags, joined only by local files and commands.
Moving the helpers below into a shared module would mean a new distribution
every release has to carry, so the copies stay. A copy nobody checks is a fork
waiting to happen, because a fix made in one `report.py` and not the other is
invisible in review: neither diff shows the sibling. This is where that drift
fails instead.

Two tiers. `IDENTICAL` names definitions that are the same bytes in both
packages and must stay that way. `SAME_LOGIC` names definitions whose prose has
already diverged - one package keeps the reasoning in a docstring or a comment,
the other does not - and whose fail-closed error class differs by design, so
only the executable code is compared. The staged-write rollback inside
`write_review_pack` is compared the same way: its destinations are named for
each package's own pack files, everything between them is shared.

Standard library only, and no component is imported: the files are read and
parsed. This runs from the repository root, like the joined conformance test.
"""

from __future__ import annotations

import ast
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLOSECONTROL = ROOT / "packages" / "monthly-close-control-plane" / "closecontrol"
REVIEWREADY = ROOT / "packages" / "review-ready-gate" / "reviewready"

PAIRS = {
    "report.py": (CLOSECONTROL / "report.py", REVIEWREADY / "report.py"),
    "loader.py": (CLOSECONTROL / "loader.py", REVIEWREADY / "loader.py"),
}

IDENTICAL = {
    "report.py": (
        # The checkout-marker guard, whole.
        "CHECKOUT_MARKERS",
        "_marker_present",
        "_configured_work_tree",
        "_enclosing_repository",
        "_reject_unreachable",
        "_same_directory",
        "_ABSENT",
    ),
    "loader.py": (
        "_ACCOUNTING_NUMBER",
        "_require_columns",
        "_text",
    ),
}

SAME_LOGIC = {
    "report.py": (
        "_money",
        "_sibling_partial",
        "_swap_into_place",
        "_remove_quietly",
        "_restore_quietly",
    ),
    "loader.py": (
        "SourceSnapshot",
        "_read_csv_rows",
        "_has_control_or_format_character",
    ),
}

# Each package raises its own fail-closed error. That difference is deliberate
# and is not drift, so the names are folded together before comparing.
ERROR_CLASSES = ("ControlInputError", "GateInputError")


def _definitions(path: Path) -> dict[str, str]:
    """Every top-level definition and simple assignment, by name, as written."""
    text = path.read_text(encoding="utf-8")
    found: dict[str, str] = {}
    for node in ast.parse(text).body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            name: str | None = node.name
        elif (
            isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
        ):
            name = node.targets[0].id
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            name = node.target.id
        else:
            continue
        segment = ast.get_source_segment(text, node)
        if segment is not None:
            found[name] = segment
    return found


def _without_docstrings(tree: ast.AST) -> ast.AST:
    for node in ast.walk(tree):
        if not isinstance(
            node, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)
        ):
            continue
        first = node.body[0] if node.body else None
        if (
            len(node.body) > 1
            and isinstance(first, ast.Expr)
            and isinstance(first.value, ast.Constant)
            and isinstance(first.value.value, str)
        ):
            node.body = node.body[1:]
    return tree


def _executable(source: str) -> str:
    """The code alone: comments dropped by unparse, docstrings and error names removed."""
    rendered = ast.unparse(_without_docstrings(ast.parse(source)))
    for error_class in ERROR_CLASSES:
        rendered = rendered.replace(error_class, "PackInputError")
    return rendered


def _rollback(path: Path) -> str:
    """The staged write and its rollback, from `staged` to the statement before the return."""
    text = path.read_text(encoding="utf-8")
    writer = next(
        node
        for node in ast.parse(text).body
        if isinstance(node, ast.FunctionDef) and node.name == "write_review_pack"
    )
    start = next(
        index
        for index, statement in enumerate(writer.body)
        if isinstance(statement, ast.AnnAssign)
        and isinstance(statement.target, ast.Name)
        and statement.target.id == "staged"
    )
    body = writer.body[start:-1]
    if not body:
        raise AssertionError(f"{path} has no staged-write block to compare")
    return "\n".join(ast.unparse(statement) for statement in body)


class SharedBlockTests(unittest.TestCase):
    def test_every_pinned_name_is_still_defined_in_both_packages(self) -> None:
        """A renamed or deleted helper must fail here, not quietly stop being compared."""
        for filename, (left, right) in PAIRS.items():
            pinned = set(IDENTICAL[filename]) | set(SAME_LOGIC[filename])
            for path in (left, right):
                with self.subTest(file=filename, package=path.parent.name):
                    self.assertLessEqual(pinned, set(_definitions(path)))

    def test_the_shared_definitions_are_byte_identical(self) -> None:
        for filename, (left, right) in PAIRS.items():
            here, there = _definitions(left), _definitions(right)
            for name in IDENTICAL[filename]:
                with self.subTest(file=filename, name=name):
                    self.assertEqual(here[name], there[name])

    def test_the_shared_code_is_identical_where_only_the_prose_differs(self) -> None:
        for filename, (left, right) in PAIRS.items():
            here, there = _definitions(left), _definitions(right)
            for name in SAME_LOGIC[filename]:
                with self.subTest(file=filename, name=name):
                    self.assertEqual(_executable(here[name]), _executable(there[name]))

    def test_the_staged_write_rollback_is_identical(self) -> None:
        left, right = PAIRS["report.py"]
        self.assertEqual(_rollback(left), _rollback(right))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
