# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The CA bundle freshness check reads the real NOTICE and fails on any difference.

The check itself runs weekly against PyPI; these offline cases run on every
pull request, so a reworded NOTICE or a bundle refresh that misses NOTICE fails
here first.
"""

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "packages/xero-trial-balance-export"
SPEC = importlib.util.spec_from_file_location("check_ca_bundle", ROOT / ".github/ci/check_ca_bundle.py")
assert SPEC is not None and SPEC.loader is not None
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class CaBundleFreshnessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.notice = (PACKAGE / "NOTICE").read_text(encoding="utf-8")
        self.bundle = (PACKAGE / "data/cacert.pem").read_bytes()
        self.version = checker.recorded(self.notice)[0]

    def test_notice_records_the_embedded_bundle(self) -> None:
        self.assertIsNone(checker.check(self.notice, self.bundle, self.version, self.bundle))

    def test_a_different_upstream_bundle_fails_with_the_refresh_step(self) -> None:
        problem = checker.check(self.notice, self.bundle, "2099.1.1", self.bundle + b"\n")
        assert problem is not None
        self.assertIn(f"certifi {self.version}, but certifi 2099.1.1 ships a different bundle", problem)
        self.assertIn("'CA bundle refresh'", problem)
        contributing = (PACKAGE / "CONTRIBUTING.md").read_text(encoding="utf-8")
        self.assertIn("\n## CA bundle refresh\n", contributing)

    def test_a_bundle_notice_does_not_record_fails(self) -> None:
        changed = self.bundle + b"\n"
        problem = checker.check(self.notice, changed, self.version, changed)
        assert problem is not None
        self.assertIn("does not match the SHA-256 NOTICE records", problem)

    def test_a_reworded_notice_is_an_error(self) -> None:
        with self.assertRaises(ValueError):
            checker.recorded(self.notice.replace("Its SHA-256 is", "Its digest is"))


if __name__ == "__main__":
    unittest.main()
