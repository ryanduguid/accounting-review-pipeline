"""Keep native publication blocked until a publisher is separately approved."""

import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]


class NativeReleaseWorkflowTests(unittest.TestCase):
    def test_native_review_releases_are_explicitly_blocked(self) -> None:
        for component in ("review-ready-gate", "monthly-close-control-plane"):
            with self.subTest(component=component):
                self.check_blocked_release(component)

    def check_blocked_release(self, component: str) -> None:
        document = yaml.safe_load(
            (ROOT / f".github/workflows/release-{component}.yml").read_text()
        )
        self.assertEqual(document["permissions"], {"contents": "read"})
        self.assertEqual(set(document["jobs"]), {"release-blocked"})
        steps = document["jobs"]["release-blocked"]["steps"]
        self.assertEqual(len(steps), 1)
        self.assertEqual(set(steps[0]), {"name", "run"})
        self.assertIn("Native release publishing is not configured", steps[0]["run"])
        self.assertEqual(steps[0]["run"].strip().splitlines()[-1], "exit 1")


if __name__ == "__main__":
    unittest.main()
