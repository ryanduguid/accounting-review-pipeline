"""Require the reviewed component checks before a release can publish."""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
POLICY = "87767ec809dc7f77bcd45808219adaf67841ae7b"
# These are component jobs from successful main-branch runs, never skip-tolerant
# aggregate gates. Review the list when a component's CI contract changes.
REQUIRED = {
    "release-accounting-excel-toolkit.yml": [
        ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
        ".github/workflows/standard-library-components.yml: lint (adapters/accounting-excel-toolkit)",
        ".github/workflows/standard-library-components.yml: verify (adapters/accounting-excel-toolkit, 3.10)",
        ".github/workflows/standard-library-components.yml: verify (adapters/accounting-excel-toolkit, 3.12)",
        ".github/workflows/standard-library-components.yml: verify (adapters/accounting-excel-toolkit, 3.13)",
        ".github/workflows/joined-conformance.yml: verify",
        ".github/workflows/codeql.yml: Analyze Python"
    ],
    "release-elizabeth-anne-alexander.yml": [
        ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
        ".github/workflows/ci.yml: packages/elizabeth-anne-alexander / build",
        ".github/workflows/ci.yml: packages/elizabeth-anne-alexander / dependency-audit",
        ".github/workflows/ci.yml: packages/elizabeth-anne-alexander / lint",
        ".github/workflows/ci.yml: packages/elizabeth-anne-alexander / test (3.10)",
        ".github/workflows/ci.yml: packages/elizabeth-anne-alexander / test (3.12)",
        ".github/workflows/ci.yml: packages/elizabeth-anne-alexander / test (3.13)",
        ".github/workflows/joined-conformance.yml: verify",
        ".github/workflows/codeql.yml: Analyze Python"
    ],
    "release-evatt.yml": [
        ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
        ".github/workflows/ci.yml: packages/evatt / build",
        ".github/workflows/ci.yml: packages/evatt / dependency-audit",
        ".github/workflows/ci.yml: packages/evatt / lint",
        ".github/workflows/ci.yml: packages/evatt / test (3.10)",
        ".github/workflows/ci.yml: packages/evatt / test (3.12)",
        ".github/workflows/ci.yml: packages/evatt / test (3.13)",
        ".github/workflows/codeql.yml: Analyze Python"
    ],
    "release-monthly-close-control-plane.yml": [
        ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
        ".github/workflows/ci.yml: dependency-audit",
        ".github/workflows/ci.yml: lint",
        ".github/workflows/ci.yml: package",
        ".github/workflows/ci.yml: test (3.10)",
        ".github/workflows/ci.yml: test (3.11)",
        ".github/workflows/ci.yml: test (3.12)",
        ".github/workflows/ci.yml: test (3.13)",
        ".github/workflows/joined-conformance.yml: verify",
        ".github/workflows/codeql.yml: Analyze Python"
    ],
    "release-review-ready-gate.yml": [
        ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
        ".github/workflows/ci.yml: packages/review-ready-gate / build",
        ".github/workflows/ci.yml: packages/review-ready-gate / dependency-audit",
        ".github/workflows/ci.yml: packages/review-ready-gate / lint",
        ".github/workflows/ci.yml: packages/review-ready-gate / test (3.10)",
        ".github/workflows/ci.yml: packages/review-ready-gate / test (3.12)",
        ".github/workflows/ci.yml: packages/review-ready-gate / test (3.13)",
        ".github/workflows/joined-conformance.yml: verify",
        ".github/workflows/codeql.yml: Analyze Python"
    ],
    "release-xero-trial-balance-export.yml": [
        ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
        ".github/workflows/ci.yml: packages/xero-trial-balance-export / build",
        ".github/workflows/ci.yml: packages/xero-trial-balance-export / dependency-audit",
        ".github/workflows/ci.yml: packages/xero-trial-balance-export / lint",
        ".github/workflows/ci.yml: packages/xero-trial-balance-export / test (3.10)",
        ".github/workflows/ci.yml: packages/xero-trial-balance-export / test (3.12)",
        ".github/workflows/ci.yml: packages/xero-trial-balance-export / test (3.13)",
        ".github/workflows/joined-conformance.yml: verify",
        ".github/workflows/codeql.yml: Analyze Python"
    ]
}


class ReleaseChecksTests(unittest.TestCase):
    def test_every_release_caller_requires_its_component_checks(self) -> None:
        workflows = ROOT / ".github" / "workflows"
        callers = sorted(
            path.name for path in workflows.glob("release*") if path.suffix in {".yml", ".yaml"}
        )
        self.assertEqual(callers, sorted(REQUIRED))
        for filename, expected in REQUIRED.items():
            with self.subTest(workflow=filename):
                text = (workflows / filename).read_text(encoding="utf-8")
                job = text.split("\n  release:\n", 1)[1].split("\n  pypi:", 1)[0]
                self.assertRegex(
                    job,
                    r"(?m)^    uses: ryanduguid/release-policy/\.github/workflows/"
                    r"release-(?:python|archive|skills)\.yml@" + POLICY + r"$",
                )
                permissions = job.split("    permissions:\n", 1)[1].split(
                    "    uses:", 1
                )[0]
                self.assertRegex(permissions, r"(?m)^      actions: read(?: #.*)?$")
                match = re.search(
                    r"^      required-checks: \|\n((?:        [^\n]*\n)+)",
                    job,
                    re.MULTILINE,
                )
                self.assertIsNotNone(match, "missing explicit component checks")
                assert match is not None
                actual = [line.strip() for line in match.group(1).splitlines()]
                self.assertCountEqual(actual, expected)
                self.assertEqual(
                    len(actual), len(set(actual)), "duplicate check selector"
                )
                for selector in actual:
                    path, name = selector.split(": ", 1)
                    self.assertTrue((ROOT / path).is_file(), path)
                    self.assertFalse(name.endswith(" / gates"), name)

    def test_push_runs_are_not_path_filtered(self) -> None:
        # The policy reads the push run of the exact main commit, and a check
        # skipped by a path filter blocks the release, so the selecting
        # workflows must not diff a push against github.event.before.
        workflows = ROOT / ".github" / "workflows"
        for filename in (
            "ci.yml",
            "ci-package.yml",
            "joined-conformance.yml",
            "standard-library-components.yml",
        ):
            with self.subTest(workflow=filename):
                text = (workflows / filename).read_text(encoding="utf-8")
                self.assertNotIn("github.event.before", text)


if __name__ == "__main__":
    unittest.main()
