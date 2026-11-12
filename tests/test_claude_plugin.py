# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The Claude Code marketplace installs plugins that exist, at their component's version.

`/plugin install` reads `.claude-plugin/marketplace.json` at the repository root,
follows each plugin's relative `source` and loads the skills under it. A moved
directory or a version bump that skips the plugin manifest would only show up
when someone installs it, so this test holds both to the tree.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MARKETPLACE = ROOT / ".claude-plugin" / "marketplace.json"


class ClaudePluginTests(unittest.TestCase):
    def test_each_listed_plugin_exists_with_its_components_version_and_skills(self) -> None:
        plugins = json.loads(MARKETPLACE.read_text(encoding="utf-8"))["plugins"]
        self.assertTrue(plugins)
        for entry in plugins:
            with self.subTest(plugin=entry["name"]):
                self.assertRegex(entry["source"], r"^\./")
                source = ROOT / entry["source"]
                manifest = json.loads(
                    (source / ".claude-plugin" / "plugin.json").read_text(encoding="utf-8")
                )
                self.assertEqual(manifest["name"], entry["name"])
                version = (source / "VERSION").read_text(encoding="utf-8").strip()
                self.assertEqual(manifest["version"], version)
                skills = sorted((source / "skills").iterdir())
                self.assertTrue(skills)
                for skill in skills:
                    self.assertTrue((skill / "SKILL.md").is_file(), skill)


if __name__ == "__main__":
    unittest.main()
