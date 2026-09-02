#!/usr/bin/env python3
"""Tests for tools/check_vba_encoding.py.

Standard library only, no test dependency. Run from the repo root:

    python -m unittest discover -s tests
"""

from __future__ import annotations

import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TOOL_PATH = REPO_ROOT / "tools" / "check_vba_encoding.py"

# The tool is a script, not a package, so load it by path rather than adding
# tools/ to sys.path and relying on import order.
_spec = importlib.util.spec_from_file_location("check_vba_encoding", TOOL_PATH)
assert _spec is not None and _spec.loader is not None
guard = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(guard)

# Written as an escape, never as a literal byte: this file stays pure ASCII.
EM_DASH = chr(0x2014)

CLEAN_BAS = b'Attribute VB_Name = "modClean"\r\nOption Explicit\r\n'


def dirty_source(name: str) -> bytes:
    """VBE-exported text carrying one UTF-8 em dash in a comment."""
    body = "' tolerance %s wide\r\n" % EM_DASH
    return ('Attribute VB_Name = "%s"\r\n' % name).encode("ascii") + body.encode(
        "utf-8"
    )


class TempVbaDir:
    """A throwaway vba/ directory to point the collector at."""

    def __enter__(self) -> Path:
        self._tmp = tempfile.TemporaryDirectory()
        return Path(self._tmp.name)

    def __exit__(self, *exc_info: object) -> None:
        self._tmp.cleanup()


def names(paths: list[Path]) -> list[str]:
    return sorted(p.name for p in paths)


class TestSuffixCoverage(unittest.TestCase):
    def test_every_vbe_text_export_is_collected(self) -> None:
        with TempVbaDir() as vba:
            for name in ("modA.bas", "clsB.cls", "frmC.frm"):
                (vba / name).write_bytes(CLEAN_BAS)
            self.assertEqual(
                names(guard.collect_targets([], vba)),
                ["clsB.cls", "frmC.frm", "modA.bas"],
            )

    def test_binary_frx_companion_is_not_collected(self) -> None:
        # A form export writes frmC.frx alongside frmC.frm. The .frx is binary
        # by design; checking it would fail every form on non-ASCII bytes.
        with TempVbaDir() as vba:
            (vba / "frmC.frm").write_bytes(CLEAN_BAS)
            (vba / "frmC.frx").write_bytes(b"\x00\x01\xff\xfe")
            self.assertEqual(names(guard.collect_targets([], vba)), ["frmC.frm"])

    def test_unrelated_files_are_not_collected(self) -> None:
        with TempVbaDir() as vba:
            (vba / "modA.bas").write_bytes(CLEAN_BAS)
            (vba / "README.md").write_bytes(
                ("notes %s here\n" % EM_DASH).encode("utf-8")
            )
            (vba / "sub").mkdir()
            self.assertEqual(names(guard.collect_targets([], vba)), ["modA.bas"])

    def test_suffix_match_ignores_case(self) -> None:
        with TempVbaDir() as vba:
            (vba / "clsB.CLS").write_bytes(CLEAN_BAS)
            self.assertEqual(names(guard.collect_targets([], vba)), ["clsB.CLS"])


class TestNonAsciiIsCaught(unittest.TestCase):
    def test_non_ascii_cls_fails(self) -> None:
        """The defect: an em dash in a .cls used to pass with exit 0."""
        with TempVbaDir() as vba:
            (vba / "modClean.bas").write_bytes(CLEAN_BAS)
            (vba / "clsProbe.cls").write_bytes(dirty_source("clsProbe"))
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                with self.assertRaises(guard.EncodingCheckError) as caught:
                    guard.run([], vba)
            self.assertIn("1 of 2 file(s)", str(caught.exception))

    def test_non_ascii_cls_exits_1_with_error_line(self) -> None:
        with TempVbaDir() as vba:
            (vba / "clsProbe.cls").write_bytes(dirty_source("clsProbe"))
            out, err = io.StringIO(), io.StringIO()
            with redirect_stdout(out), redirect_stderr(err):
                status = guard.main([], vba)
            self.assertEqual(status, 1)
            # Detail and summary share stderr: split across the two streams, a
            # redirected capture showed them out of order or lost one.
            self.assertIn("3 non-ASCII byte(s)", err.getvalue())
            self.assertIn("clsProbe.cls", err.getvalue())
            self.assertIn("error: ", err.getvalue())
            self.assertNotIn("FAIL", out.getvalue())

    def test_non_ascii_frm_fails(self) -> None:
        """Asserting only that SOME EncodingCheckError is raised passed under
        the exact defect it claims to cover: drop .frm from VBE_TEXT_SUFFIXES
        and collect_targets raises "no .bas, .cls or .frm files found", which
        is the same exception type. The file has to be named as the failure."""
        with TempVbaDir() as vba:
            (vba / "frmProbe.frm").write_bytes(dirty_source("frmProbe"))
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                with self.assertRaises(guard.EncodingCheckError) as caught:
                    guard.run([], vba)
            self.assertIn("1 of 1 file(s)", str(caught.exception))
            self.assertIn("frmProbe.frm", str(guard.collect_targets([], vba)))
            self.assertIn("3 non-ASCII byte(s)", " | ".join(
                guard.check_file(vba / "frmProbe.frm")
            ))

    def test_a_subdirectory_export_is_checked_not_skipped(self) -> None:
        """The VBE's Export File... dialog remembers a folder, so source
        filed one level down is normal. iterdir walked the top level only and
        passed those files with exit 0 while never opening them."""
        with TempVbaDir() as vba:
            (vba / "modTop.bas").write_bytes(CLEAN_BAS)
            nested = vba / "forms"
            nested.mkdir()
            (nested / "frmNested.frm").write_bytes(dirty_source("frmNested"))
            found = guard.collect_targets([], vba)
            self.assertIn("frmNested.frm", " | ".join(str(p) for p in found))
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                with self.assertRaises(guard.EncodingCheckError) as caught:
                    guard.run([], vba)
            self.assertIn("1 of 2 file(s)", str(caught.exception))

    def test_bom_and_line_endings_still_checked_on_cls(self) -> None:
        with TempVbaDir() as vba:
            (vba / "clsB.cls").write_bytes(guard.UTF8_BOM + b"Option Explicit\n")
            problems = guard.check_file(vba / "clsB.cls")
            joined = " | ".join(problems)
            self.assertIn("UTF-8 BOM", joined)
            self.assertIn("bare LF", joined)

    def test_bare_cr_line_endings_are_caught_and_fail_the_run(self) -> None:
        """The other half of the CRLF claim, and the only branch of
        check_bytes with no test. A .bas written with CR-only endings - an
        old Mac editor, or a botched conversion - imports into the VBE as one
        line, so the module the user believed was verified is destroyed. It
        is pure ASCII with no BOM and no bare LF, so this branch is the only
        thing standing between it and "pass: pure ASCII, CRLF, no BOM"."""
        with TempVbaDir() as vba:
            (vba / "modCR.bas").write_bytes(
                b'Attribute VB_Name = "modCR"\rOption Explicit\r'
            )
            joined = " | ".join(guard.check_file(vba / "modCR.bas"))
            self.assertIn("2 bare CR (no following LF)", joined)
            self.assertNotIn("bare LF", joined)
            self.assertNotIn("non-ASCII", joined)
            out, err = io.StringIO(), io.StringIO()
            with redirect_stdout(out), redirect_stderr(err):
                self.assertEqual(guard.main([], vba), 1)
            self.assertIn("bare CR", err.getvalue())
            self.assertNotIn("file(s) pass", out.getvalue())

    def test_a_trailing_cr_at_end_of_file_is_caught(self) -> None:
        # The end-of-buffer arm of the same branch: a CR as the final byte
        # has no following byte to inspect.
        self.assertIn(
            "1 bare CR (no following LF)",
            " | ".join(guard.check_bytes(b"Option Explicit\r\nEnd Sub\r")),
        )
        self.assertEqual(guard.check_bytes(CLEAN_BAS), [])


class TestEmptyDirectoryStillFailsLoudly(unittest.TestCase):
    def test_empty_directory_raises(self) -> None:
        with TempVbaDir() as vba:
            with self.assertRaises(guard.EncodingCheckError) as caught:
                guard.collect_targets([], vba)
            self.assertIn(".bas, .cls or .frm", str(caught.exception))

    def test_directory_with_only_unrelated_files_raises(self) -> None:
        # A rename from .bas to .txt must not silently empty the guard.
        with TempVbaDir() as vba:
            (vba / "modA.txt").write_bytes(CLEAN_BAS)
            with self.assertRaises(guard.EncodingCheckError):
                guard.collect_targets([], vba)

    def test_missing_directory_raises(self) -> None:
        with TempVbaDir() as vba:
            with self.assertRaises(guard.EncodingCheckError) as caught:
                guard.collect_targets([], vba / "gone")
            self.assertIn("no vba directory at", str(caught.exception))

    def test_missing_directory_exits_1_without_traceback(self) -> None:
        with TempVbaDir() as vba:
            err = io.StringIO()
            with redirect_stdout(io.StringIO()), redirect_stderr(err):
                status = guard.main([], vba / "gone")
            self.assertEqual(status, 1)
            self.assertTrue(err.getvalue().startswith("error: "))
            self.assertNotIn("Traceback", err.getvalue())


class TestCleanTreePasses(unittest.TestCase):
    def test_clean_temp_dir_passes(self) -> None:
        with TempVbaDir() as vba:
            (vba / "modA.bas").write_bytes(CLEAN_BAS)
            (vba / "clsB.cls").write_bytes(CLEAN_BAS)
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(guard.run([], vba), 0)
            self.assertIn("2 file(s) pass", out.getvalue())

    def test_repo_vba_directory_passes(self) -> None:
        """Self-check: the tracked VBA source is importable as written."""
        with redirect_stdout(io.StringIO()):
            self.assertEqual(guard.main([]), 0)


class TestExplicitArguments(unittest.TestCase):
    def test_named_file_is_checked_whatever_its_suffix(self) -> None:
        with TempVbaDir() as vba:
            probe = vba / "clsProbe.cls"
            probe.write_bytes(dirty_source("clsProbe"))
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                with self.assertRaises(guard.EncodingCheckError):
                    guard.run([str(probe)])

    def test_named_missing_file_raises(self) -> None:
        with TempVbaDir() as vba:
            with self.assertRaises(guard.EncodingCheckError) as caught:
                guard.collect_targets([str(vba / "nope.cls")])
            self.assertIn("no such file", str(caught.exception))


class TestSourceIsAscii(unittest.TestCase):
    def test_tool_and_tests_carry_no_non_ascii_byte(self) -> None:
        for path in (TOOL_PATH, Path(__file__).resolve()):
            data = path.read_bytes()
            offsets = [i for i, b in enumerate(data) if b > guard.MAX_ASCII]
            self.assertEqual(offsets, [], "%s has non-ASCII bytes" % path)


if __name__ == "__main__":
    unittest.main()
