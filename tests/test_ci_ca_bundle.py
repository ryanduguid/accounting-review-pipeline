# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The CA bundle freshness check reads the real NOTICE and fails on any difference.

The check itself runs weekly against PyPI; these offline cases run on every
pull request, so a reworded NOTICE or a bundle refresh that misses NOTICE fails
here first.
"""

import contextlib
import hashlib
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import zipfile

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

    def test_fetch_requires_https_and_uses_a_bounded_request(self) -> None:
        with self.assertRaisesRegex(ValueError, "not HTTPS"):
            checker.fetch("http://fixture.example/wheel.whl")
        with patch.object(checker.urllib.request, "urlopen", return_value=io.BytesIO(b"wheel")) as fetch:
            self.assertEqual(checker.fetch("https://fixture.example/wheel.whl"), b"wheel")
        fetch.assert_called_once_with("https://fixture.example/wheel.whl", timeout=60)

    def wheel(self, bundle=b"synthetic certificate bundle"):
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            archive.writestr("certifi/cacert.pem", bundle)
        return output.getvalue()

    def release(self, wheel):
        return {"info": {"version": "fixture-version"}, "urls": [{
            "packagetype": "bdist_wheel", "url": "https://fixture.example/wheel.whl",
            "digests": {"sha256": hashlib.sha256(wheel).hexdigest()},
        }]}

    def test_current_release_checks_the_wheel_digest_before_reading_its_bundle(self) -> None:
        wheel = self.wheel()
        release = self.release(wheel)
        with patch.object(checker, "fetch", side_effect=[json.dumps(release).encode(), wheel]) as fetch:
            self.assertEqual(checker.current_release(),
                             ("fixture-version", b"synthetic certificate bundle"))
        self.assertEqual([call.args[0] for call in fetch.call_args_list],
                         [checker.PYPI, "https://fixture.example/wheel.whl"])
        release["urls"][0]["digests"]["sha256"] = "0" * 64
        with patch.object(checker, "fetch", side_effect=[json.dumps(release).encode(), wheel]):
            with self.assertRaisesRegex(ValueError, "does not match the SHA-256"):
                checker.current_release()

    def test_missing_or_ambiguous_wheels_fail_before_any_download(self) -> None:
        for count in (0, 2):
            with self.subTest(count=count):
                release = self.release(self.wheel())
                release["urls"] *= count
                with patch.object(checker, "fetch", return_value=json.dumps(release).encode()) as fetch:
                    with self.assertRaisesRegex(ValueError, f"found {count}"):
                        checker.current_release()
                fetch.assert_called_once_with(checker.PYPI)

    def test_main_reports_a_match_and_refuses_a_changed_bundle(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary)
            (package / "data").mkdir()
            (package / "NOTICE").write_text(self.notice, encoding="utf-8")
            (package / "data/cacert.pem").write_bytes(self.bundle)
            output = io.StringIO()
            with patch.object(checker, "current_release", return_value=(self.version, self.bundle)):
                with contextlib.redirect_stdout(output):
                    checker.main([str(package)])
            self.assertIn(f"matches the current certifi release, {self.version}", output.getvalue())
            with patch.object(checker, "current_release", return_value=("changed", b"different")):
                with self.assertRaisesRegex(SystemExit, "ships a different bundle"):
                    checker.main([str(package)])

    def test_cli_argument_and_missing_file_errors_are_single_lines(self) -> None:
        script = ROOT / ".github/ci/check_ca_bundle.py"
        with tempfile.TemporaryDirectory() as temporary:
            for arguments, expected in (([], "FileNotFoundError:"),
                                        (["first", "second"], "ValueError:")):
                with self.subTest(arguments=arguments):
                    result = subprocess.run([sys.executable, str(script), *arguments],
                                            cwd=temporary, capture_output=True, text=True, timeout=10)
                    self.assertEqual(result.returncode, 1)
                    self.assertEqual(result.stdout, "")
                    self.assertEqual(len(result.stderr.splitlines()), 1)
                    self.assertTrue(result.stderr.startswith(expected), result.stderr)


if __name__ == "__main__":
    unittest.main()
