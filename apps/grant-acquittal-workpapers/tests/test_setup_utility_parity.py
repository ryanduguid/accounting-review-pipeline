# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.

"""The recorded command-line cases that setup_utility.py and its Rust port share.

tests/fixtures/setup_utility_cli.json holds each case's exit code and output as
setup_utility.py produces them; examples/setup_utility.rs checks the same file.
"""

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import setup_utility

FIXTURE = Path(__file__).with_name("fixtures") / "setup_utility_cli.json"


def run_case(arguments):
    """Run one case in a fresh fake pipeline; return (exit code, stdout, stderr)."""
    with tempfile.TemporaryDirectory(prefix="setup parity ") as temp:
        root = Path(temp).resolve()
        source = root / "pipeline" / setup_utility.GRANT_PROJECT
        source.mkdir(parents=True)
        driver = root / "pipeline" / setup_utility.DRIVER
        driver.parent.mkdir(parents=True)
        driver.write_text("")
        (root / "existing").mkdir()
        argv = [argument.replace("{root}", root.as_posix()) for argument in arguments]
        out, err = io.StringIO(), io.StringIO()
        with (patch.object(setup_utility, "SOURCE", source),
              patch.object(setup_utility.sys, "version_info", (3, 14)),
              patch.object(setup_utility.shutil, "which", side_effect=lambda name: name),
              patch.object(sys, "argv", ["setup_utility.py"]),
              patch.dict("os.environ", {"NO_COLOR": "1"}),
              contextlib.redirect_stdout(out), contextlib.redirect_stderr(err)):
            try:
                code = setup_utility.main(argv)
            except SystemExit as stop:
                code = stop.code
        return code, out.getvalue(), err.getvalue()


class SetupUtilityParityTests(unittest.TestCase):
    def test_recorded_cases_match_the_python_entry_point(self):
        cases = json.loads(FIXTURE.read_text(encoding="utf-8"))["cases"]
        self.assertGreaterEqual(len(cases), 8)
        for case in cases:
            with self.subTest(case["name"]):
                self.assertEqual(run_case(case["args"]), (case["exit"], case["stdout"], case["stderr"]))


if __name__ == "__main__":
    unittest.main()
