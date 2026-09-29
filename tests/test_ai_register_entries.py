"""Each supplier record names the version its component currently carries.

docs/ai-register-entries.md is supplier information firms use in their own AI
registers, and each table describes the version named in its Name and version row. The
table for review-ready-gate still named 0.1.8 after 0.1.9 changed the gate's
stated limits, so this test reads every Name and version row and holds it to the
component's own version.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENTRIES = ROOT / "docs" / "ai-register-entries.md"
NAME_ROW = re.compile(
    r"^\| Name and version \| \[[^\]]+\]\(\.\./packages/([a-z0-9-]+)/README\.md\)[^|]*? (\d+\.\d+\.\d+),",
    re.M,
)


def package_version(directory: str) -> str:
    """Read a static version, or follow setuptools' dynamic ``attr`` to its module."""
    project = (ROOT / "packages" / directory / "pyproject.toml").read_text(encoding="utf-8")
    static = re.search(r'^version = "([^"]+)"$', project, re.M)
    if static is not None:
        return static.group(1)
    dynamic = re.search(r'^version = \{ attr = "([\w.]+)\.(\w+)" \}$', project, re.M)
    if dynamic is None:
        raise AssertionError(f"packages/{directory}/pyproject.toml declares no version")
    module, name = dynamic.groups()
    source = (ROOT / "packages" / directory / Path(*module.split("."))).with_suffix(".py")
    assigned = re.search(rf'^{name} = "([^"]+)"$', source.read_text(encoding="utf-8"), re.M)
    if assigned is None:
        raise AssertionError(f"{source} does not assign {name}")
    return assigned.group(1)


class AiRegisterEntryTests(unittest.TestCase):
    def test_every_entry_names_its_components_current_version(self) -> None:
        rows = NAME_ROW.findall(ENTRIES.read_text(encoding="utf-8"))
        self.assertEqual(
            sorted(directory for directory, _ in rows),
            ["elizabeth-anne-alexander", "evatt", "review-ready-gate"],
        )
        for directory, version in rows:
            with self.subTest(component=directory):
                self.assertEqual(version, package_version(directory))


if __name__ == "__main__":
    unittest.main()
