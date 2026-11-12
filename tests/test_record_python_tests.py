# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The developer recorder preserves calls and refuses unrepresentable fixtures."""

from dataclasses import dataclass
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "packages/evatt/tools/record_python_tests.py"


class RecorderTests(unittest.TestCase):
    def setUp(self):
        name = "arp_recorder_contract"
        spec = importlib.util.spec_from_file_location(name, SCRIPT)
        self.tool = importlib.util.module_from_spec(spec)
        sys.modules[name] = self.tool
        self.addCleanup(sys.modules.pop, name, None)
        spec.loader.exec_module(self.tool)
        self.tool.pytest_runtest_setup(types.SimpleNamespace(name="fabricated_case"))

    def wrap(self, function, module_name="patterns", name="structured_spans"):
        module = types.ModuleType("evatt." + module_name)
        setattr(module, name, function)
        alias = types.ModuleType("evatt.caller")
        alias.call = function
        alias.unrelated = lambda: "unrelated"
        self.tool.MODULES = [module, alias]
        self.tool.capture(module, name)
        self.assertIs(alias.call, getattr(module, name))
        self.assertEqual(alias.unrelated(), "unrelated")
        return alias.call

    def records(self):
        return list(self.tool.RECORDS.values())

    def test_serialisation_preserves_supported_types_and_rejects_objects(self):
        @dataclass
        class Count:
            count: int

        value = {"items": (b"\x00\xff", True, 3, 2.5, None, "text"), "count": Count(4)}
        self.assertEqual(self.tool.serial(value), {
            "items": [{"hex": "00ff"}, True, 3, 2.5, None, "text"], "count": {"count": 4},
        })
        with self.assertRaises(TypeError):
            self.tool.serial(object())

    def test_alias_calls_keep_inputs_results_and_one_deduplicated_record(self):
        function = self.wrap(lambda text, *, suffix: text.encode() + suffix)
        for _ in range(2):
            self.assertEqual(function("fabricated", suffix=b"!"), b"fabricated!")
        self.assertEqual(self.records(), [{
            "mode": "patterns.structured_spans", "args": ["fabricated"],
            "kwargs": {"suffix": {"hex": "21"}}, "outcome": {"ok": {"hex": "6661627269636174656421"}},
            "test": "fabricated_case",
        }])
        self.assertEqual(self.tool.NESTED, 0)

    def test_nested_calls_record_only_the_outer_operation(self):
        inner = self.wrap(lambda value: value + "!")
        outer = self.wrap(lambda value: inner(value), name="person_name_spans")
        self.assertEqual(outer("sample"), "sample!")
        self.assertEqual(len(self.records()), 1)
        self.assertEqual(self.records()[0]["mode"], "patterns.person_name_spans")
        self.assertEqual(self.tool.NESTED, 0)

    def test_halt_details_are_recorded_and_the_original_error_propagates(self):
        class Halt(ValueError):
            unknowns = [{"value": "fabricated", "kind": "person"}]

        error = Halt("review required")

        def halted(value):
            raise error

        self.tool.HALT_TYPE = Halt
        function = self.wrap(halted, module_name="redact", name="redact")
        with self.assertRaises(Halt) as caught:
            function("source")
        self.assertIs(caught.exception, error)
        self.assertEqual(self.records()[0]["outcome"], {
            "error": "Halt", "message": "review required", "unknowns": Halt.unknowns,
        })
        self.assertEqual(self.tool.NESTED, 0)

    def test_unsupported_returns_inputs_and_filtered_cases_do_not_create_fixtures(self):
        unsupported = object()
        self.assertIs(self.wrap(lambda: unsupported)(), unsupported)
        self.assertEqual(self.records(), [])
        function = self.wrap(lambda *args, **kwargs: None)
        for active, args, kwargs in (
            ("", ("text",), {}),
            ("test_monkeypatch", ("text",), {}),
            ("test_never_reports_a_placeholder", ("text",), {}),
            ("ordinary", ("x" * 5001,), {}),
            ("ordinary", (unsupported,), {}),
            ("ordinary", ("text",), {"value": unsupported}),
        ):
            with self.subTest(active=active, length=len(args)):
                self.tool.ACTIVE = active
                self.assertIsNone(function(*args, **kwargs))
                self.assertEqual(self.records(), [])
                self.assertEqual(self.tool.NESTED, 0)

    def test_map_inputs_are_captured_as_text_and_paths_are_redacted(self):
        def invalid_map(path):
            raise ValueError(f"invalid map: {path}")

        function = self.wrap(invalid_map, module_name="entities", name="load")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "map.json"
            path.write_text('{"fabricated":true}', encoding="utf-8")
            with self.assertRaises(ValueError):
                function(path)
            self.assertEqual(self.records()[0]["args"], ['{"fabricated":true}'])
            self.assertEqual(self.records()[0]["outcome"]["message"], "invalid map: <map>")
            self.tool.RECORDS.clear()
            for invalid in (path.with_name("absent.json"), path):
                if invalid == path:
                    path.write_bytes(b"\xff")
                with self.assertRaises(ValueError):
                    function(invalid)
                self.assertEqual(self.records(), [])

    def test_session_output_filters_removed_api_shapes_and_migrates_version_bytes(self):
        payload = b'{"tool_version": "0.2.0","nested":{"tool_version":"0.2.0"}}'
        valid = {"mode": "redact.redact", "args": ["text", [{
            "value": "person", "placeholder": "P1", "kind": "person", "added": "fixture",
        }]], "kwargs": {}, "outcome": {"ok": {"hex": payload.hex()}}, "test": "valid"}
        invalid = {**valid, "args": ["text", [{
            "value": 7, "placeholder": "P1", "kind": "person", "added": "fixture",
        }]]}
        nested = {"mode": "patterns.structured_spans", "args": [[1, {"hex": payload.hex()}]],
                  "kwargs": {}, "outcome": {"ok": None}, "test": "nested"}
        self.tool.RECORDS = {"valid": valid, "invalid": invalid, "nested": nested}
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "recorded.json"
            self.tool.OUTPUT = str(output)
            self.tool.pytest_sessionfinish(None, 0)
            rows = json.loads(output.read_text())
        self.assertEqual([row["test"] for row in rows], ["valid", "nested"])
        expected = payload.replace(b"0.2.0", b"1.0.0")
        self.assertEqual(bytes.fromhex(rows[0]["outcome"]["ok"]["hex"]), expected)
        self.assertEqual(bytes.fromhex(rows[1]["args"][0][1]["hex"]), expected)

    def test_main_passes_the_selected_oracle_and_returns_pytest_failure(self):
        fields = {
            "patterns": ["structured_spans", "person_name_spans", "_ambiguous_identifiers"],
            "entities": ["_fold", "load"], "redact": ["redact", "residual"],
            "restore": ["restore"], "verify": ["findings"],
            "disclosure": ["create_record", "check_record"],
        }
        modules = {}
        for name, names in fields.items():
            module = modules[name] = types.ModuleType("evatt." + name)
            for field in names:
                def operation(value=None):
                    return value
                setattr(module, field, operation)
        modules["redact"].Halt = ValueError
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "oracle"
            output = Path(temporary) / "recorded.json"
            with (
                patch.object(sys, "path", sys.path.copy()),
                patch.object(sys, "argv", [str(SCRIPT), "--python-root", str(root), "--output", str(output)]),
                patch.object(self.tool.importlib, "import_module", side_effect=lambda name: modules[name.split(".")[-1]]),
                patch.object(self.tool.pytest, "main", return_value=7) as pytest_main,
            ):
                self.assertEqual(self.tool.main(), 7)
                self.assertEqual(sys.path[0], str(root))
            pytest_main.assert_called_once_with([str(root / "tests")], plugins=[self.tool])
            self.assertEqual(self.tool.OUTPUT, str(output))

    def test_cli_help_does_not_require_an_external_oracle(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--help"],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--python-root", result.stdout)
        self.assertIn("--output", result.stdout)


if __name__ == "__main__":
    unittest.main()
