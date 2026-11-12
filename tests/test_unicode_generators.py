# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""Check generated Unicode contracts with the real supported interpreter."""

import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import re
import runpy
import sys
import unicodedata
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
EVATT = ROOT / "packages/evatt"
FROZEN_UNICODE = sys.version_info[:2] == (3, 14) and unicodedata.unidata_version == "16.0.0"


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def generated(path):
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        namespace = runpy.run_path(str(path))
    return namespace, output.getvalue()


class WorkpaperUnicodeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.paths = [ROOT / component / "tools/gen_unicode_tables.py" for component in (
            "packages/elizabeth-anne-alexander", "apps/grant-acquittal-workpapers")]
        cls.tables = [generated(path) for path in cls.paths]

    def test_tables_document_the_actual_database_and_known_unicode_boundaries(self):
        for _, text in self.tables:
            with self.subTest(header=text.splitlines()[0]):
                self.assertIn(sys.version.split()[0], text)
                self.assertIn(unicodedata.unidata_version, text)
                for declaration in ("PY_SPACE", "PY_NOT_PRINTABLE", "DECIMAL_ZEROS"):
                    self.assertIn("pub const " + declaration, text)
                for zero in ("0x0030", "0x0660", "0x0966", "0x1D7CE"):
                    self.assertIn(zero + ",", text)
                self.assertIn("(0x0009, 0x000D)", text)
                self.assertIn("(0x3000, 0x3000)", text)
        grant = self.tables[1][1]
        self.assertIn('(0x2028, 0x2028, "Zl")', grant)
        self.assertIn('(0x2029, 0x2029, "Zp")', grant)

    def test_runs_retain_a_final_scalar_and_preserve_values(self):
        eaa, grant = (item[0] for item in self.tables)
        self.assertEqual(eaa["ranges"](lambda ch: ord(ch) == 0x10FFFF), [(0x10FFFF, 0x10FFFF)])
        self.assertEqual(grant["runs"](lambda ch: ord(ch) == 0x10FFFF),
                         [(0, 0x10FFFE, False), (0x10FFFF, 0x10FFFF, True)])
        self.assertEqual(grant["category_class"]("A"), "L")
        self.assertEqual(grant["category_class"]("\u2028"), "Zl")
        self.assertEqual(grant["category_class"]("\u2029"), "Zp")

    def test_inconsistent_decimal_database_is_refused_before_emitting_digit_tables(self):
        decimal = unicodedata.decimal
        for path in self.paths:
            for bad_character, message in (("1", "do not run 0 to 9"),
                                           ("0", "do not cover every")):
                def inconsistent(character, default=None):
                    return None if character == bad_character else decimal(character, default)
                with self.subTest(generator=str(path), character=bad_character):
                    with patch.object(unicodedata, "decimal", new=inconsistent):
                        with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(SystemExit, message):
                            runpy.run_path(str(path))


class EvattUnicodeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tables = load(EVATT / "tools/gen_unicode_tables.py", "evatt_unicode_generator_contract")

    def test_scalar_ranges_and_regex_classes_keep_boundaries_and_omit_surrogates(self):
        tables = self.tables
        self.assertEqual(tables.ranges(lambda ch: ord(ch) == tables.LIMIT - 1),
                         [(tables.LIMIT - 1, tables.LIMIT - 1)])
        self.assertEqual(tables.ranges(lambda ch: ch in "ABZ"), [(65, 66), (90, 90)])
        self.assertEqual(tables.regex_class([90, 66, 0xD800, 65]), r"[\x{41}-\x{42}\x{5A}]")
        self.assertEqual(tables.regex_class([]), "[]")
        self.assertIn(48, tables.decimal_zeros())
        self.assertIn(0x660, tables.decimal_zeros())

    @unittest.skipIf(FROZEN_UNICODE, "unsupported-interpreter contract runs on the other matrix interpreter")
    def test_generators_refuse_a_different_interpreter_or_unicode_database(self):
        with patch.object(sys, "argv", ["gen_unicode_tables.py"]):
            with contextlib.redirect_stderr(io.StringIO()) as error, self.assertRaises(SystemExit) as result:
                self.tables.main()
        self.assertEqual(result.exception.code, 2)
        self.assertIn("requires CPython 3.14 with Unicode 16.0.0", error.getvalue())
        cases = load(EVATT / "tools/gen_unicode_cases.py", "evatt_unicode_case_contract")
        with self.assertRaises(AssertionError):
            cases.main()

    @unittest.skipUnless(FROZEN_UNICODE, "requires actual CPython 3.14 and Unicode 16")
    def test_generated_tables_have_verified_ignorecase_closure_and_body_digest(self):
        tables = self.tables
        groups = tables.ignorecase_groups()
        self.assertEqual(set(groups[ord("K")]), {ord("K"), ord("k"), ord("\u212a")})
        with contextlib.redirect_stderr(io.StringIO()) as verification:
            tables.verify(groups)
        self.assertIn("verified against compiled re", verification.getvalue())
        with patch.object(sys, "argv", ["gen_unicode_tables.py", "--test"]):
            with contextlib.redirect_stderr(io.StringIO()):
                tables.main()
        with patch.object(sys, "argv", ["gen_unicode_tables.py"]):
            with contextlib.redirect_stdout(io.StringIO()) as output:
                runpy.run_path(str(EVATT / "tools/gen_unicode_tables.py"), run_name="__main__")
        text = output.getvalue()
        header, body = text.split("\n\n", 1)
        self.assertIn(hashlib.sha256(body.encode()).hexdigest(), header)
        self.assertIn("pub const RE_IGNORECASE:", body)
        self.assertIn("pub const PY_CASE_IGNORABLE:", body)

    @unittest.skipUnless(FROZEN_UNICODE, "requires actual CPython 3.14 and Unicode 16")
    def test_generated_cases_reproduce_the_frozen_fabricated_fixture(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            runpy.run_path(str(EVATT / "tools/gen_unicode_cases.py"), run_name="__main__")
        document = json.loads(output.getvalue())
        expected = json.loads((EVATT / "tests/fixtures/unicode-16.json").read_text())
        self.assertEqual(document.pop("interpreter"), sys.version.split()[0])
        expected.pop("interpreter")
        self.assertEqual(document, expected)


class EvattPatternTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        tables = load(EVATT / "tools/gen_unicode_tables.py", "evatt_pattern_unicode_contract")
        cls.tables = tables
        with patch.dict(sys.modules, {"gen_unicode_tables": tables}):
            cls.patterns = load(EVATT / "tools/gen_patterns.py", "evatt_pattern_generator_contract")

    def test_translations_preserve_flags_groups_repeats_assertions_and_refuse_unsupported_ops(self):
        patterns = self.patterns
        parser, c = patterns.parser, patterns.c
        for expression in (r"(?P<word>a)(b)(?:x|y)[^z]", r"(?i:(?-i:A))", r"a{2}b{1,3}?c{2,}+",
                           r"(?<=a)(?<!b)(?=c)(?!d)", r"(?>a)(a)\1", r"\A\d\b\B\Z", r"^[a-z]$"):
            parsed = parser.parse(expression)
            names = {value: key for key, value in parsed.state.groupdict.items()}
            with self.subTest(expression=expression):
                self.assertTrue(patterns.translate(parsed, False, names))
        self.assertEqual(patterns.literal(0xD800, False), "(?!)")
        self.assertEqual(patterns.literal(ord("A"), False), r"\x{41}")
        self.assertIn(r"\x{41}", patterns.literal(ord("a"), True))
        self.assertIn(r"\x{30}-\x{39}", patterns.translate([(c.CATEGORY, c.CATEGORY_DIGIT)], False, {}))
        self.assertIn(r"\x{41}", patterns.char_class([(c.RANGE, (65, 66))], True))
        for kind, opposite in ((c.CATEGORY_WORD, c.CATEGORY_NOT_WORD),
                               (c.CATEGORY_SPACE, c.CATEGORY_NOT_SPACE),
                               (c.CATEGORY_DIGIT, c.CATEGORY_NOT_DIGIT)):
            self.assertFalse(patterns.category(kind) & patterns.category(opposite))
            self.assertEqual(patterns.category(kind) | patterns.category(opposite), patterns.SCALARS)
        with self.assertRaises(ValueError):
            patterns.char_class([(c.ANY, None)], False)
        with self.assertRaises(ValueError):
            patterns.translate([(c.ANY, None)], False, {})

    @unittest.skipUnless(FROZEN_UNICODE, "requires actual CPython 3.14 and Unicode 16")
    def test_frozen_pattern_output_matches_the_committed_rust_constants(self):
        with patch.dict(sys.modules, {"gen_unicode_tables": self.tables}), contextlib.redirect_stdout(io.StringIO()) as output:
            runpy.run_path(str(EVATT / "tools/gen_patterns.py"), run_name="__main__")
        generated_text = output.getvalue()
        committed = (EVATT / "src/regexes.rs").read_text()
        rows = r'\(\s*"([^"]+)"\s*,\s*r###"(.*?)"###\s*,?\s*\)'
        self.assertEqual(dict(re.findall(rows, generated_text, re.S)), dict(re.findall(rows, committed, re.S)))
        constants = r'pub const (WORD|JOIN_ATOM): &str = r###"(.*?)"###;'
        self.assertEqual(dict(re.findall(constants, generated_text, re.S)), dict(re.findall(constants, committed, re.S)))
        self.assertIn("_JOIN_SPLIT", generated_text)


if __name__ == "__main__":
    unittest.main()
