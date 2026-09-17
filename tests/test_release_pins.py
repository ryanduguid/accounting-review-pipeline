"""The Release Policy pin the callers run is the one the documentation claims.

README.md describes the current callers' pin in the present tense, and each
RELEASING.md verification recipe derives the signer digest from the caller at
the tagged commit. Both drift silently when a caller is repointed, so this
test reads the callers and holds the documents to them.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
PIN = re.compile(r"uses: ryanduguid/release-policy/\.github/workflows/[a-z-]+\.yml@([0-9a-f]{40})$", re.M)
CLAIM = re.compile(
    r"through a root caller pinned to the independently reviewed Release Policy commit\n"
    r"`([0-9a-f]{40})`"
)
CURRENT_CALLERS = re.compile(r"Every root release caller now pins `([0-9a-f]{7})`")


def caller_pins() -> dict[str, str]:
    pins: dict[str, str] = {}
    for workflow in sorted(WORKFLOWS.glob("release-*.yml")):
        match = PIN.search(workflow.read_text(encoding="utf-8"))
        if match is None:
            raise AssertionError(f"{workflow.name} does not pin release-policy by full commit")
        pins[workflow.name] = match.group(1)
    return pins


class ReleasePinTests(unittest.TestCase):
    def test_every_release_caller_pins_a_full_commit(self) -> None:
        pins = caller_pins()
        self.assertEqual(len(pins), 6, sorted(pins))

    def test_readme_names_the_pin_the_callers_run(self) -> None:
        """One claim, one pin: if the callers diverge, the README has to say so."""
        pins = set(caller_pins().values())
        self.assertEqual(len(pins), 1, "callers run different pins; rewrite the README claim")
        claim = CLAIM.search((ROOT / "README.md").read_text(encoding="utf-8"))
        self.assertIsNotNone(claim, "README.md no longer states the callers' pin")
        assert claim is not None
        self.assertEqual({claim.group(1)}, pins)

    def test_imports_names_the_pin_the_callers_run(self) -> None:
        pins = set(caller_pins().values())
        claim = CURRENT_CALLERS.search((ROOT / "IMPORTS.md").read_text(encoding="utf-8"))
        self.assertIsNotNone(claim, "IMPORTS.md no longer states the callers' current pin")
        assert claim is not None
        self.assertEqual({pin[:7] for pin in pins}, {claim.group(1)})

    def test_releasing_recipes_derive_the_signer_digest_from_the_caller(self) -> None:
        """A literal digest in a recipe is the pin of some earlier day, not of the release."""
        recipes = sorted(ROOT.glob("*/*/RELEASING.md"))
        self.assertEqual(len(recipes), 6, recipes)
        pins = set(caller_pins().values())
        for recipe in recipes:
            text = recipe.read_text(encoding="utf-8")
            with self.subTest(recipe=str(recipe.relative_to(ROOT))):
                for pin in pins:
                    self.assertNotIn(f"--signer-digest {pin}", text, "literal current pin")
                self.assertNotIn("--signer-digest fcf25e5", text, "superseded pin")
                self.assertIn('--signer-digest "$policy_sha"', text)
                self.assertIn("?ref=$release_commit", text)


if __name__ == "__main__":
    unittest.main()
