"""Regression checks for the synthetic journal boundary and review consumers."""
from __future__ import annotations

import ctypes
import html
import importlib.util
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from contextlib import ExitStack, redirect_stderr
from decimal import Decimal
from html.parser import HTMLParser
from pathlib import Path
from unittest.mock import MagicMock, patch

APP = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("review_workflow", APP / "tools/review_workflow.py")
if spec is None or spec.loader is None:
    raise ImportError("Review workflow loader is unavailable.")
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)


def manifest_fixture(command: str, launcher: bytes = b"original") -> dict:
    distribution, package, entry = workflow.PRODUCERS[command]
    return {"manifest_version": 1, "command": command,
            "launcher_name": command + (".exe" if os.name == "nt" else ""),
            "launcher_sha256": workflow.digest(launcher),
            "interpreter": {"implementation": "cpython", "version": "3.14.7", "cache_tag": "cpython-314", "executable_sha256": "0" * 64},
            "distribution": {"name": distribution, "version": "0.1.0", "requires_python": ">=3.10", "entry_point": entry},
            "package": {"name": package, "files": {"__init__.py": "0" * 64, "py.typed": workflow.digest(b"")}}}


class ReportElements(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.elements: list[tuple[str, dict[str, str | None]]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.elements.append((tag, dict(attrs)))


class ReviewWorkflowTests(unittest.TestCase):
    def resource_api(self, payloads: dict[str, bytes], *, handle: int = 5) -> MagicMock:
        api = MagicMock()
        api.LoadLibraryExW.return_value = handle
        api.FreeLibrary.return_value = 1
        names = {name: index + 1 for index, name in enumerate(payloads)}
        buffers = {names[name]: ctypes.create_string_buffer(value) for name, value in payloads.items()}
        api.FindResourceW.side_effect = lambda module, name, kind: names.get(name, 0)
        api.SizeofResource.side_effect = lambda module, resource: len(payloads[next(name for name, index in names.items() if index == resource)])
        api.LoadResource.side_effect = lambda module, resource: resource
        api.LockResource.side_effect = lambda resource: ctypes.addressof(buffers[resource])
        return api

    def test_native_resource_reader_maps_only_data_and_releases_it(self) -> None:
        payloads = {"UV_TRAMPOLINE_KIND": b"\x01", "UV_PYTHON_PATH": b"C:/fixture/python.exe", "UV_SCRIPT_DATA": b"fixture"}
        api = self.resource_api(payloads)
        with patch("ctypes.WinDLL", return_value=api, create=True) as loader:
            self.assertEqual(workflow.windows_resources(Path("fixture.exe")), payloads)
        loader.assert_called_once_with("kernel32", use_last_error=True)
        api.LoadLibraryExW.assert_called_once_with("fixture.exe", None, 0x22)
        self.assertEqual([call.args[2] for call in api.FindResourceW.call_args_list], [10, 10, 10])
        api.FreeLibrary.assert_called_once_with(5)

    def test_resource_reader_refuses_invalid_handles_sizes_and_pointers(self) -> None:
        for handle, payloads, null_pointer in ((0, {}, False), (4, {}, False), (5, {"UV_TRAMPOLINE_KIND": b""}, False),
                                               (5, {"UV_TRAMPOLINE_KIND": b"\x01\x01"}, False),
                                               (5, {"UV_PYTHON_PATH": b"x" * 131073}, False),
                                               (5, {"UV_TRAMPOLINE_KIND": b"\x01"}, True)):
            api = self.resource_api(payloads, handle=handle)
            if null_pointer:
                api.LockResource.side_effect = lambda resource: None
            with self.subTest(handle=handle, sizes={name: len(value) for name, value in payloads.items()}), patch("ctypes.WinDLL", return_value=api, create=True):
                with self.assertRaises((ValueError, OSError)):
                    workflow.windows_resources(Path("fixture.exe"))
                if handle:
                    api.FreeLibrary.assert_called_once_with(handle)
                else:
                    api.FreeLibrary.assert_not_called()

    def test_resource_reader_reports_release_failure_and_absent_resources(self) -> None:
        api = self.resource_api({})
        with patch("ctypes.WinDLL", return_value=api, create=True):
            self.assertEqual(workflow.windows_resources(Path("fixture.exe")), dict.fromkeys(("UV_TRAMPOLINE_KIND", "UV_PYTHON_PATH", "UV_SCRIPT_DATA")))
        api.FreeLibrary.assert_called_once_with(5)
        api = self.resource_api({})
        api.FreeLibrary.return_value = 0
        with patch("ctypes.WinDLL", return_value=api, create=True), self.assertRaisesRegex(OSError, "released"):
            workflow.windows_resources(Path("fixture.exe"))

    def test_private_package_names_are_refused_before_any_file_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".env").touch()
            with patch.object(Path, "read_bytes", side_effect=AssertionError("Private file bytes were accessed.")), self.assertRaisesRegex(ValueError, "excluded private file"):
                workflow.package_files(root)

    def test_child_environment_removes_shadow_paths_and_uses_empty_bytecode_cache(self) -> None:
        with patch.object(workflow.os, "environ", {"PYTHONPATH": "shadow", "PYTHONHOME": "shadow", "PATH": "fixture-path"}):
            with workflow.producer_environment() as environment:
                self.assertNotIn("PYTHONPATH", environment)
                self.assertNotIn("PYTHONHOME", environment)
                self.assertEqual(environment["PYTHONNOUSERSITE"], "1")
                self.assertEqual(environment["PYTHONDONTWRITEBYTECODE"], "1")
                cache = Path(environment["PYTHONPYCACHEPREFIX"])
                self.assertEqual(list(cache.iterdir()), [])
            self.assertFalse(cache.exists())

    @unittest.skipIf(os.name == "nt", "POSIX interpreter leaf-link contract")
    def test_interpreter_leaf_links_are_bounded_and_parent_links_are_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            target = root / "python-target"
            target.write_bytes(b"regular executable fixture")
            link = root / "python"
            link.symlink_to(target.name)
            self.assertEqual(workflow.interpreter_file(link), (link, target))
            link.unlink()
            link.symlink_to("other")
            other = root / "other"
            other.symlink_to(link.name)
            with self.assertRaisesRegex(ValueError, "link loop"):
                workflow.interpreter_file(link)
            linked_parent = root / "linked-parent"
            linked_parent.symlink_to(root, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "Linked paths"):
                workflow.interpreter_file(linked_parent / target.name)

    @unittest.skipIf(os.name == "nt", "POSIX console-script formats")
    def test_posix_direct_and_quoted_shell_wrappers_bind_the_declared_interpreter(self) -> None:
        body = b'from closecontrol.cli import main\nmain()\n'
        for source, expected in ((b'#!/fixture/python\n' + body, "/fixture/python"),
                                 (b'#!/bin/sh\n\'\'\'exec\' "/fixture with spaces/python" "$0" "$@"\n\' \'\'\'\n' + body, "/fixture with spaces/python")):
            self.assertEqual(workflow.declared_interpreter(Path("unused"), source, "close-control"), Path(expected))
        for source in (b'#!/usr/bin/env python\n' + body, b'#!relative/python\n' + body,
                       b'#!/bin/sh\nexec /fixture/python\n' + body):
            with self.assertRaises(ValueError):
                workflow.declared_interpreter(Path("unused"), source, "close-control")

    def test_model_account_mapping_matches_the_pinned_case(self) -> None:
        case, _ = workflow.sources()
        model = (APP / "australian-accounting-power-bi.SemanticModel/definition/tables/Review_Evidence.tmdl").read_text()
        mapping = model.split("AccountMap = #table", 1)[1].split("Run = Review_Run", 1)[0]
        pairs = re.findall(r'\{"([^"]+)", "([^"]+)"\}', mapping)
        self.assertEqual(len(pairs), len(case["account_ids"]))
        self.assertEqual(dict(pairs), case["account_ids"])

    def test_ambiguous_and_deep_json_are_controlled_cli_refusals(self) -> None:
        values = (b'{"files":{},"files":{}}', b'{"case":{"entity":"one","entity":"two"}}',
                  b'[]', b'null', b'{"schema_version":true}', b'{"files":null}',
                  b'{"case":[]}', b'{"invocations":{}}', b'{"period":12}',
                  b'{"value":NaN}', b'{"value":Infinity}', b'{"value":1e999}', b'\xff',
                  b'{"value":' + b'[' * 128 + b'0' + b']' * 128 + b'}')
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory)
            for data in values:
                with self.subTest(data=data[:60]):
                    (run / "receipt.json").write_bytes(data)
                    (run / "receipt.sha256").write_text(workflow.digest(data), encoding="ascii")
                    result = subprocess.run([sys.executable, "-B", str(APP / "tools/review_workflow.py"), "verify", "--run", str(run)], capture_output=True, timeout=30)
                    self.assertEqual(result.returncode, 1)
                    self.assertIn(b"Review workflow refused:", result.stderr)
                    self.assertNotIn(b"Traceback", result.stderr)

    def test_json_strings_do_not_count_as_structure_and_utf8_bom_is_accepted(self) -> None:
        document = {"quoted": '[{\\"' * 100}
        self.assertEqual(workflow.strict_json(b'\xef\xbb\xbf' + workflow.json_bytes(document), "Fixture"), document)

    def test_consumed_pack_shapes_are_validated_before_indexing(self) -> None:
        value: object
        for validator, values in ((workflow.close_document, ([], None, {"overall_status": "REVIEW", "source_sha256": []})),
                                  (workflow.driver_document, ([], None, {"source_sha256": {}, "accounts": [None]})),
                                  (workflow.readiness_document, (None, {"overall_status": "READY", "controls_not_run": None})),
                                  (workflow.comparison_document, ([], {"review_boundary": "boundary", "findings": None}))):
            for value in values:
                with self.subTest(validator=validator.__name__, value=value), self.assertRaises(ValueError):
                    validator(value)

    def test_driver_numeric_fields_require_finite_text_and_integer_counts(self) -> None:
        account = {"tenant": "fixture", "account_id": "account", "movement": "1.00", "transactions_total": "1.00", "unexplained": "0.00", "transactions_in_window": 1,
                   "drivers": [{"TransactionID": "J:1", "Date": "2024-09-30", "Reference": "J", "Description": "Fixture", "Amount": "1.00"}]}
        source_hashes = {"transactions": "0" * 64, **{"pack:" + name: "0" * 64 for name in workflow.CLOSE_FILES}}
        baseline = {"source_sha256": source_hashes, "accounts": [account]}
        workflow.driver_document(baseline)
        for field, value in (("movement", "NaN"), ("unexplained", "Infinity"), ("transactions_total", "bad"),
                             ("movement", "1E+0"), ("transactions_total", "+1.00"), ("unexplained", "0.000"),
                             ("movement", " 1.00"), ("movement", "1.00 "),
                             ("transactions_in_window", True), ("transactions_in_window", "1"), ("transactions_in_window", -1)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                workflow.driver_document({"source_sha256": source_hashes, "accounts": [account | {field: value}]})

    def close_fixture(self) -> dict:
        findings = workflow.rows((APP / "samples/sample-review-exceptions.csv").read_bytes())
        item = findings[0]
        return {"overall_status": "REVIEW", "source_sha256": {name: "0" * 64 for name in
                ("current_trial_balance", "prior_trial_balance")}, "current_report_dates": ["2024-09-30"],
                "prior_report_dates": ["2024-08-31"], "controls_not_run": [], "acknowledgement": None,
                "thresholds": {"absolute_variance": "1000.00", "percentage_variance": "10.00%"},
                "exceptions": [{"control": item["Control"], "tenant": item["Tenant"], "account_id": item["AccountID"],
                    "account_name": item["Account"], "status": item["Status"], "current_value": item["Current"],
                    "prior_value": item["Prior"], "difference": item["Difference"], "threshold": item["Threshold"],
                    "reason": item["Reason"], "reviewer_action": item["Action"]}],
                "client_queries": [{"query_id": "Q-fixture", "control": item["Control"], "tenant": item["Tenant"],
                    "account_id": item["AccountID"], "question": item["Question"], "evidence_requested": item["EvidenceRequested"]}]}

    def test_close_query_identities_and_readiness_records_are_unambiguous(self) -> None:
        doc = self.close_fixture()
        workflow.close_document(doc)
        query = doc["client_queries"][0]
        for duplicate in (query | {"query_id": "Q-other", "question": "Conflicting question"},
                          query | {"account_id": "different-account"}):
            with self.subTest(query=duplicate), self.assertRaises(ValueError):
                workflow.close_document(doc | {"client_queries": [query, duplicate]})
        control = {"slot": "fixture", "filename": "fixture.csv", "reason": "Not supplied"}
        workflow.readiness_document({"overall_status": "READY", "controls_not_run": [control]})
        for invalid in (None, 1, {"slot": 1}, control | {"reason": ""}, control | {"extra": "unwitnessed"}):
            with self.subTest(control=invalid), self.assertRaises(ValueError):
                workflow.readiness_document({"overall_status": "READY", "controls_not_run": [invalid]})

    def test_fixed_case_money_matches_the_model_display_grammar(self) -> None:
        doc = self.close_fixture()
        for text in ("NaN", "Infinity", "1E+3", "+1000.00", "1000.000", " 1000.00", "1000.00 ", "1_000.00"):
            for field in ("current_value", "prior_value", "difference", "threshold"):
                with self.subTest(field=field, text=text), self.assertRaises(ValueError):
                    workflow.close_document(doc | {"exceptions": [doc["exceptions"][0] | {field: text}]})
            with self.subTest(threshold=text), self.assertRaises(ValueError):
                workflow.close_document(doc | {"thresholds": doc["thresholds"] | {"absolute_variance": text}})
        for text in ("123456789012345678901234567890.00", "-0.00", "0001.00"):
            accepted = doc | {"exceptions": [doc["exceptions"][0] | {"difference": text}]}
            self.assertIs(workflow.close_document(accepted), accepted)
        for percentage in ("NaN%", "1E+1%", "10.00", " 10.00%"):
            with self.subTest(percentage=percentage), self.assertRaises(ValueError):
                workflow.close_document(doc | {"thresholds": doc["thresholds"] | {"percentage_variance": percentage}})

    def test_comparison_labels_require_the_producer_side_invariants(self) -> None:
        record = {"account_name": "Fixture", "account_id": "account", "control": "period_variance",
                  "current_value": "1.00", "status": "REVIEW"}
        altered = record | {"current_value": "2.00"}
        for group in ("findings", "queries"):
            def document(change, previous, current):
                side = (lambda value: [] if value is None else [value]) if group == "findings" else (lambda value: value)
                item = {"change": change, "previous": side(previous), "current": side(current),
                        "control": "period_variance", "account_id": "account", "query_id": "Q-fixture"}
                return {"review_boundary": "Fixture", "scope_changes": [], "findings": [], "queries": [], group: [item]}
            for change, previous, current in (("NEW", None, record), ("CHANGED", record, altered),
                    ("RECURRING", record, record), ("NOT_RAISED", record, None), ("NOT_COMPARABLE", record, None)):
                valid = document(change, previous, current)
                if change == "NOT_COMPARABLE":
                    valid["scope_changes"] = ["thresholds"]
                workflow.comparison_document(valid)
            for change, previous, current in (("NEW", record, None), ("NEW", record, record),
                    ("CHANGED", record, record), ("CHANGED", None, record), ("RECURRING", record, altered),
                    ("RECURRING", None, record), ("NOT_RAISED", record, record), ("NOT_COMPARABLE", None, record)):
                with self.subTest(group=group, change=change, previous=previous, current=current), self.assertRaises(ValueError):
                    workflow.comparison_document(document(change, previous, current))

    def test_comparison_scope_and_repeated_group_order_follow_the_producer(self) -> None:
        record = {"account_name": "Fixture", "account_id": "account", "control": "period_variance",
                  "current_value": "1.00", "status": "REVIEW"}
        for group in ("findings", "queries"):
            previous = [record] if group == "findings" else record
            current: list[dict[str, str]] | None = [] if group == "findings" else None
            item = {"control": "period_variance", "account_id": "account", "query_id": "Q-fixture",
                    "previous": previous, "current": current}
            for change, scope in (("NOT_COMPARABLE", []), ("NOT_RAISED", ["thresholds"]),
                                  ("NOT_COMPARABLE", [""]), ("NOT_COMPARABLE", [" "])):
                document = {"review_boundary": "Fixture", "scope_changes": scope, "findings": [], "queries": [],
                            group: [item | {"change": change}]}
                with self.subTest(group=group, change=change, scope=scope), self.assertRaises(ValueError):
                    workflow.comparison_document(document)
        altered = record | {"current_value": "2.00"}
        ordered = sorted([record, altered], key=lambda row: json.dumps(row, sort_keys=True))
        item = {"control": "period_variance", "account_id": "account", "previous": ordered,
                "current": list(reversed(ordered))}
        document = {"review_boundary": "Fixture", "scope_changes": [], "queries": [],
                    "findings": [item | {"change": "CHANGED"}]}
        with self.assertRaises(ValueError):
            workflow.comparison_document(document)
        workflow.comparison_document(document | {"findings": [item | {"change": "RECURRING"}]})

    def test_invalid_comparison_is_a_controlled_html_cli_refusal(self) -> None:
        record = {"account_name": "Fixture", "account_id": "account", "control": "period_variance",
                  "current_value": "1.00", "status": "REVIEW"}
        comparison = {"review_boundary": "Fixture", "scope_changes": [], "queries": [], "findings": [
            {"control": "period_variance", "account_id": "account", "change": "NOT_COMPARABLE",
             "previous": [record], "current": []}]}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "refused.html"
            stderr = io.StringIO()
            with redirect_stderr(stderr), self.assertRaises(SystemExit) as refused:
                self.render_comparison(comparison, cli_output=output)
            self.assertEqual(refused.exception.code, 1)
            self.assertIn("Review workflow refused:", stderr.getvalue())
            self.assertNotIn("Traceback", stderr.getvalue())
            self.assertFalse(output.exists())

    def test_entrypoint_script_rejects_side_effects_and_wrong_targets(self) -> None:
        good = b'import sys\nfrom closecontrol.cli import main\nif __name__ == "__main__":\n    sys.exit(main())\n'
        workflow.launcher_script(good, "close-control")
        pypa = b'import re\nimport sys\nif __name__ == "__main__":\n    from closecontrol.cli import main\n    sys.argv[0] = re.sub(r"(-script\\.pyw|\\.exe)?$", "", sys.argv[0])\n    sys.exit(main())\n'
        workflow.launcher_script(pypa, "close-control")
        for source in (good.replace(b"closecontrol", b"reviewready"), good + b'print("side effect")\n',
                       good.replace(b"main()", b'main("argument")'), good.replace(b"import sys", b"import subprocess"),
                       pypa.replace(b'    sys.exit', b'    print("side effect")\n    sys.exit'),
                       pypa.replace(b'closecontrol.cli import main', b'closecontrol.cli import main as other')):
            with self.subTest(source=source), self.assertRaises(ValueError):
                workflow.launcher_script(source, "close-control")

    def test_package_manifest_includes_empty_marker_and_detects_file_population(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "__init__.py").write_bytes(b"# Fixture\n")
            (root / "py.typed").write_bytes(b"")
            original = workflow.package_files(root)
            self.assertEqual(original["py.typed"], workflow.digest(b""))
            (root / "added.py").write_bytes(b"# Added\n")
            self.assertNotEqual(workflow.package_files(root), original)
            (root / "added.py").unlink()
            (root / "py.typed").unlink()
            self.assertNotEqual(workflow.package_files(root), original)

    def test_shared_case_preserves_all_six_pinned_sources(self) -> None:
        case, source = workflow.sources()
        self.assertEqual(len(source), 6)
        self.assertEqual({name: workflow.digest(data) for name, data in source.items()}, case["source_sha256"])

    def test_current_and_prior_tie_to_independently_grouped_source_lines(self) -> None:
        case, source = workflow.sources()
        current, prior, transactions = workflow.trial_balances(case, source, 9)
        ledger = workflow.rows(source["sample-general-ledger.csv"])
        for data, period in ((current, "2024-09-30"), (prior, "2024-08-31")):
            for row in workflow.rows(data):
                included = [r for r in ledger if r["EntityID"] == "ENT001" and r["AccountCode"] == row["AccountCode"] and "2024-07-01" <= r["PostingDate"] <= period]
                for side in ("Debit", "Credit"):
                    self.assertEqual(Decimal(row["YTD" + side]), sum((Decimal(r[side]) for r in included), Decimal(0)))
                    self.assertEqual(Decimal(row[side]), sum((Decimal(r[side]) for r in included if r["PostingDate"][:7] == period[:7]), Decimal(0)))
            self.assertEqual(sum((Decimal(r["Debit"]) - Decimal(r["Credit"]) for r in workflow.rows(data)), Decimal(0)), 0)
        actual = workflow.rows(transactions)
        eligible = [r for r in ledger if r["EntityID"] == "ENT001" and "2024-07-01" <= r["PostingDate"] <= "2024-09-30"]
        self.assertEqual({r["TransactionID"] for r in actual}, {r["JournalID"] + ":" + r["LineNumber"] for r in eligible})

    def test_movement_bridge_and_original_opening_are_exact(self) -> None:
        case, source = workflow.sources()
        current, prior, _ = workflow.trial_balances(case, source, 9)
        before = {r["AccountID"]: r for r in workflow.rows(prior)}
        for row in workflow.rows(current):
            opening = Decimal(before[row["AccountID"]]["YTDDebit"]) - Decimal(before[row["AccountID"]]["YTDCredit"])
            self.assertEqual(opening + Decimal(row["Debit"]) - Decimal(row["Credit"]), Decimal(row["YTDDebit"]) - Decimal(row["YTDCredit"]))
        original = workflow.rows(source["sample-general-ledger.csv"])
        prototype = next(r for r in original if r["EntityID"] == "ENT001")
        reversal = [prototype | {"JournalID": "TEST-REVERSAL", "LineNumber": str(index), "PostingDate": "2024-09-30", "Debit": debit, "Credit": credit, "Amount": amount} for index, debit, credit, amount in ((1, "35.00", "0.00", "35.00"), (2, "0.00", "35.00", "-35.00"))]
        modified = source | {"sample-general-ledger.csv": workflow.csv_bytes(list(original[0]), original + reversal)}
        altered, _, _ = workflow.trial_balances(case, modified, 9)
        before = next(r for r in workflow.rows(current) if r["AccountCode"] == prototype["AccountCode"])
        after = next(r for r in workflow.rows(altered) if r["AccountCode"] == prototype["AccountCode"])
        for side in ("Debit", "Credit"):
            self.assertEqual(Decimal(after[side]) - Decimal(before[side]), Decimal("35.00"))

    def test_unsupported_financial_year_is_refused(self) -> None:
        case, source = workflow.sources()
        case["financial_year_start"] = "2026-07-01"
        with self.assertRaisesRegex(ValueError, "August and September 2024"):
            workflow.trial_balances(case, source, 9)

    def test_removed_duplicated_and_unbalanced_lines_are_refused(self) -> None:
        case, source = workflow.sources()
        original = workflow.rows(source["sample-general-ledger.csv"])
        for changes in (original[1:], original + [original[0]], [original[0] | {"Debit": "350000.01"}, *original[1:]]):
            changed = source | {"sample-general-ledger.csv": workflow.csv_bytes(list(original[0]), changes)}
            with self.assertRaises(ValueError):
                workflow.trial_balances(case, changed, 9)

    def test_sources_detect_one_byte_change(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory)
            (app / "samples").mkdir()
            case, source = workflow.sources()
            for name, data in source.items():
                (app / "samples" / name).write_bytes(data)
            (app / "samples/shared-review-case.json").write_bytes(workflow.json_bytes(case))
            workflow.sources(app)
            (app / "samples/sample-general-ledger.csv").write_bytes(source["sample-general-ledger.csv"] + b"\n")
            with self.assertRaisesRegex(ValueError, "Pinned fabricated source changed"):
                workflow.sources(app)

    def test_network_and_nas_paths_are_refused_before_access(self) -> None:
        values = (r"z:\private", r"Z:\private", r"Z:private", "Z:/private",
                  r"\\office-nas\ryan\private", "//office-nas/ryan/private",
                  r"\\?\Z:\private", r"\\?\C:\private", r"\\?\UNC\office-nas\ryan\private",
                  r"\\.\C:\private", "//?/Z:/private", "//?/C:/private")
        for value in values:
            with self.subTest(value=value), ExitStack() as stack:
                for method in ("absolute", "exists", "is_symlink", "stat", "lstat", "resolve"):
                    stack.enter_context(patch.object(Path, method, side_effect=AssertionError("Filesystem access attempted.")))
                with self.assertRaisesRegex(ValueError, "local directory"):
                    workflow.ordinary(Path(value))

    def test_native_absolutisation_retains_prohibited_location_refusal(self) -> None:
        for drive, text in (("Z:", r"Z:\private"), ("", r"\\office-nas\ryan\private")):
            normalised = MagicMock(drive=drive, **{"__str__.return_value": text})
            with self.subTest(drive=drive, text=text), ExitStack() as stack:
                absolute = stack.enter_context(patch.object(Path, "absolute", return_value=normalised))
                for method in ("exists", "is_symlink", "stat", "lstat", "resolve"):
                    stack.enter_context(patch.object(Path, method, side_effect=AssertionError("Filesystem access attempted.")))
                with self.assertRaisesRegex(ValueError, "local directory"):
                    workflow.ordinary(Path("relative"))
                absolute.assert_called_once_with()

    def assert_link_refused_before_child(self, link: Path, candidate: Path) -> None:
        inspected = []
        native_lstat = Path.lstat

        def checked_lstat(path: Path) -> os.stat_result:
            if path.name == "new-review.html":
                raise AssertionError("Child metadata accessed before link refusal.")
            inspected.append(path)
            return native_lstat(path)

        with patch.object(Path, "lstat", autospec=True, side_effect=checked_lstat), \
                patch.object(Path, "resolve", side_effect=AssertionError("Resolution attempted before link refusal.")):
            with self.assertRaisesRegex(ValueError, "Linked paths are not admitted"):
                workflow.ordinary(candidate)
        self.assertIn(link, inspected)

    def test_real_symlinks_are_refused_before_child_access(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            target = root / "target"
            target.mkdir()
            for name, destination in (("broken-link", root / "missing-target"), ("directory-link", target)):
                link = root / name
                try:
                    link.symlink_to(destination, target_is_directory=True)
                except OSError as error:
                    if os.name == "nt":
                        self.skipTest(f"Windows symlink creation unavailable: {error}")
                    raise
                try:
                    for candidate in (link / "new-review.html",
                                      root / "missing" / ".." / name / "new-review.html",
                                      root / "missing" / "deeper" / ".." / ".." / name / "new-review.html"):
                        with self.subTest(link=name, candidate=str(candidate)):
                            self.assert_link_refused_before_child(link, candidate)
                finally:
                    link.unlink()

    @unittest.skipUnless(os.name == "nt", "Windows junction regression")
    def test_real_windows_junction_is_refused_before_child_access(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            target = root / "target"
            target.mkdir()
            marker = target / "preserved.txt"
            marker.write_text("Target retained.", encoding="utf-8")
            link = root / "junction"
            script = root / "create-junction.ps1"
            script.write_text("param([string]$Link, [string]$Target)\n$ErrorActionPreference = 'Stop'\n"
                              "New-Item -ItemType Junction -Path $Link -Target $Target | Out-Null\n", encoding="utf-8")
            subprocess.run(["powershell.exe", "-NoProfile", "-File", str(script),
                            "-Link", str(link), "-Target", str(target)], check=True, capture_output=True)
            try:
                for candidate in (link / "new-review.html", root / "missing" / ".." / "junction" / "new-review.html",
                                  root / "missing" / "deeper" / ".." / ".." / "junction" / "new-review.html"):
                    with self.subTest(candidate=str(candidate)):
                        self.assert_link_refused_before_child(link, candidate)
            finally:
                link.rmdir()
            self.assertEqual(marker.read_text(encoding="utf-8"), "Target retained.")

    def test_missing_subtree_parent_climb_preserves_local_destination(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            candidate = root / "missing" / "deeper" / ".." / ".." / "new-review.html"
            self.assertEqual(workflow.new_output(candidate), root / "new-review.html")

    def test_metadata_permission_errors_propagate(self) -> None:
        with patch.object(Path, "lstat", side_effect=PermissionError("Metadata denied.")):
            with self.assertRaisesRegex(PermissionError, "Metadata denied"):
                workflow.ordinary(Path("new-review.html"))

    def test_new_local_output_outside_version_control_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "new-review.html"
            self.assertEqual(workflow.new_output(destination), destination.resolve())

    def test_output_inside_repository_or_existing_destination_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "outside version control"):
            workflow.new_output(APP / "must-not-create")
        with self.assertRaisesRegex(ValueError, "must be new"):
            workflow.new_output(APP)

    def test_review_island_has_no_relationship_to_financial_model(self) -> None:
        definition = APP / "australian-accounting-power-bi.SemanticModel/definition"
        text = (definition / "relationships.tmdl").read_text()
        blocks = text.split("relationship ")
        selected = [block for block in blocks if "Review_" in block]
        self.assertEqual(len(selected), 1)
        self.assertIn("fromColumn: Review_Evidence.ExceptionKey", selected[0])
        self.assertIn("toColumn: Review_Exception.ExceptionKey", selected[0])
        self.assertNotIn("BothDirections", selected[0])
        for path in (definition / "tables").glob("Review_*.tmdl"):
            text = path.read_text()
            self.assertNotIn("dataType: double", text)
            self.assertNotIn("SUM(", text)

    def test_drillthrough_binds_complete_context_and_uses_back(self) -> None:
        report = APP / "australian-accounting-power-bi.Report/definition/pages"
        page = json.loads((report / "a02closeevid202410002/page.json").read_text())
        self.assertEqual(page["type"], "Drillthrough")
        properties = {r["fieldExpr"]["Column"]["Property"] for r in page["pageBinding"]["parameters"]}
        self.assertEqual(properties, {"RunID", "Entity", "Period", "Basis", "ExceptionKey"})
        back = json.loads((report / "a02closeevid202410002/visuals/reviewevidback00001/visual.json").read_text())
        self.assertEqual(back["visual"]["visualContainerObjects"]["visualLink"][0]["properties"]["type"]["expr"]["Literal"]["Value"], "'Back'")
        finding = json.loads((report / "a01closehome202410001/visuals/reviewhomefinding01/visual.json").read_text())
        self.assertEqual(finding["visual"]["query"]["queryState"]["Values"]["projections"][0]["field"]["Column"]["Property"], "Account")
        self.assertEqual(finding["visual"]["objects"]["selection"][0]["properties"]["singleSelect"]["expr"]["Literal"]["Value"], "true")

    def test_portable_html_escapes_every_dynamic_value(self) -> None:
        attack = '<script>alert("fixture")</script></style>&'
        context = dict.fromkeys(("RunID", "Tenant", "Period", "Basis", "Currency"), attack)
        item = dict.fromkeys(("Control", "Account", "Status", "Current", "Prior", "Difference", "Threshold", "Reason", "Action", "Question", "EvidenceRequested", "EvidenceState", "ExceptionKey"), attack)
        evidence = dict.fromkeys(("ExceptionKey", "TransactionID", "Date", "Reference", "Description", "Amount"), attack)
        projected = {"sample-review-run.csv": workflow.csv_bytes(list(context), [context]), "sample-review-exceptions.csv": workflow.csv_bytes(list(item), [item]), "sample-review-evidence.csv": workflow.csv_bytes(list(evidence), [evidence])}
        receipt = {"files": {attack: attack}, "producer_manifests": {command: manifest_fixture(command) for command in workflow.PRODUCERS}}
        snapshot = {"close/close-review-pack.json": b"{}"}
        with patch.object(workflow, "verify", return_value=(receipt, snapshot)), patch.object(workflow, "projection", return_value=projected), patch.object(workflow, "pinned_producer", return_value=b""):
            output = workflow.render_html(Path("unused"), Path("unused"))
        self.assertNotIn("<script>", output)
        self.assertNotIn("</style>&", output)
        self.assertIn(html.escape(attack, quote=True), output)
        self.assertIn("default-src 'none'", output)
        self.assertNotIn("<script", output.lower())
        self.assertNotIn(" src=", output.lower())
        parsed = ReportElements()
        parsed.feed(output)
        identifiers = {attrs["id"] for _, attrs in parsed.elements if "id" in attrs}
        links = [attrs["href"] for tag, attrs in parsed.elements if tag == "a"]
        self.assertTrue(links)
        self.assertTrue(all(link and link.startswith("#") and link[1:] in identifiers for link in links))
        self.assertNotIn(attack, identifiers)

    def test_portable_findings_have_distinct_destinations_and_visible_exact_evidence(self) -> None:
        findings = workflow.rows((APP / "samples/sample-review-exceptions.csv").read_bytes())
        evidence = workflow.rows((APP / "samples/sample-review-evidence.csv").read_bytes())
        # Two findings can share an account name and must still have distinct destinations.
        findings[1]["Account"] = findings[0]["Account"]
        findings[-1]["Control"] = "unsupported_control"
        findings[-1]["EvidenceState"] = "Line-level evidence is unavailable under this control contract"
        evidence = [row for row in evidence if row["ExceptionKey"] != findings[-1]["ExceptionKey"]]
        evidence[0]["Amount"] = "-1234567890.00100"
        evidence[0]["Reference"] = 'Reference <demo> & "quoted"'
        projected = {"sample-review-run.csv": (APP / "samples/sample-review-run.csv").read_bytes(), "sample-review-exceptions.csv": workflow.csv_bytes(list(findings[0]), findings), "sample-review-evidence.csv": workflow.csv_bytes(list(evidence[0]), evidence)}
        receipt = {"files": {}, "producer_manifests": {command: manifest_fixture(command) for command in workflow.PRODUCERS}}
        with patch.object(workflow, "verify", return_value=(receipt, {"close/close-review-pack.json": b"{}"})), patch.object(workflow, "projection", return_value=projected), patch.object(workflow, "pinned_producer", return_value=b""):
            output = workflow.render_html(Path("unused"), Path("unused"))
        parsed = ReportElements()
        parsed.feed(output)
        identifiers = [attrs["id"] for _, attrs in parsed.elements if "id" in attrs]
        self.assertEqual(len(identifiers), len(set(identifiers)))
        articles = [attrs for tag, attrs in parsed.elements if tag == "article"]
        self.assertEqual(len(articles), len(findings))
        self.assertTrue(all(attrs.get("tabindex") == "-1" for attrs in articles))
        self.assertEqual(sum(tag == "tr" for tag, _ in parsed.elements), len(evidence) + sum(any(row["ExceptionKey"] == item["ExceptionKey"] for row in evidence) for item in findings))
        self.assertNotIn("<details", output)
        self.assertIn("No journal rows are supplied for this finding.", output)
        self.assertIn("Line-level evidence is unavailable under this control contract", output)
        self.assertIn(evidence[0]["Amount"], output)
        self.assertIn(html.escape(evidence[0]["Reference"], quote=True), output)
        next_links = [attrs["href"] for tag, attrs in parsed.elements if tag == "a" and (attrs.get("href") or "").startswith("#finding-")]
        for article in articles:
            self.assertIn("#" + str(article["id"]), next_links)
        self.assertNotIn(f"#finding-{len(findings) + 1}", next_links)

    def test_producer_refusal_prevents_portable_display(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            executable = bin_dir / ("close-control.exe" if workflow.os.name == "nt" else "close-control")
            executable.write_bytes(b"fabricated test executable")
            manifest = manifest_fixture("close-control", executable.read_bytes())
            receipt = {"producer_manifests": {"close-control": manifest}}
            with patch.object(workflow, "verify", return_value=(receipt, {})), patch.object(workflow, "producer_manifest", return_value=manifest), patch.object(workflow.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
                with self.assertRaisesRegex(ValueError, "Producer verification refused"):
                    workflow.render_html(Path("unused"), bin_dir)

    def render_comparison(self, comparison: dict, *, acknowledgement: bool = False, removed_input: bool = False, cli_output: Path | None = None) -> str:
        projected = {name: (APP / "samples" / name).read_bytes() for name in
                     ("sample-review-run.csv", "sample-review-exceptions.csv", "sample-review-evidence.csv")}
        previous, current = Path("previous"), Path("current")
        def verified(run):
            files = {"inputs/trial_balance.csv": "old" if run == previous else "new"}
            if run == previous and removed_input:
                files["inputs/review-note.json"] = "old note"
            return ({"case": "same fabricated case", "period": "2024-08-31" if run == previous else "2024-09-30",
                     "files": files, "producer_manifests": {command: manifest_fixture(command) for command in workflow.PRODUCERS}},
                    {"close/close-review-pack.json": workflow.json_bytes({"acknowledgement":
                     "fabricated note" if run == current and acknowledgement else None})})
        with patch.object(workflow, "verify", side_effect=verified), patch.object(workflow, "projection", return_value=projected), patch.object(workflow, "pinned_producer", return_value=workflow.json_bytes(comparison)):
            if cli_output is not None:
                with patch.object(sys, "argv", ["review_workflow.py", "html", "--run", str(current),
                                               "--previous", str(previous), "--output", str(cli_output)]):
                    workflow.main()
                return ""
            return workflow.render_html(current, Path("unused"), previous)

    def test_comparison_preserves_classifications_exact_records_and_escapes_data(self) -> None:
        attack = '<script>alert("comparison")</script>&'
        record = {"account_name": attack, "account_id": attack, "control": attack,
                  "current_value": "-1234567890.00100", "status": attack}
        finding_changes, query_changes = [], []
        for change in ("NEW", "CHANGED", "RECURRING", "NOT_RAISED", "NOT_COMPARABLE"):
            previous = [] if change == "NEW" else [record]
            current = [] if change in ("NOT_RAISED", "NOT_COMPARABLE") else [record]
            finding_changes.append({"account_id": attack, "control": attack, "change": change,
                                    "previous": previous, "current": current})
            query_changes.append({"query_id": attack, "change": change,
                                  "previous": previous[0] if previous else None,
                                  "current": current[0] if current else None})
        finding_changes[1]["current"] = [record, record | {"current_value": "0.00000", "status": "BLOCKED"},
                                         record | {"current_value": ""}]
        query_changes[1]["current"] = record | {"question": "Changed question"}
        not_raised = self.render_comparison({"findings": [finding_changes.pop(3)], "queries": [query_changes.pop(3)],
                                             "scope_changes": [], "review_boundary": "Unscoped comparison"})
        self.assertIn('data-change="NOT_RAISED">Not raised</th><td class="amount">1</td><td class="amount">1</td>', not_raised)
        comparison = {"findings": finding_changes, "queries": query_changes,
                      "scope_changes": [attack], "review_boundary": "NOT_RAISED does not mean resolved or approved."}
        output = self.render_comparison(comparison, acknowledgement=True)
        summary, full = output.split('<section id="comparison-record"', 1)
        self.assertNotIn("<script", output)
        self.assertIn(html.escape(attack, quote=True), summary)
        self.assertIn("-1234567890.00100", summary)
        self.assertIn("0.00000", summary)
        self.assertIn("Empty producer value", summary)
        self.assertIn("No finding raised in this run.", summary)
        self.assertIn("NOT_RAISED does not mean resolved or approved.", summary)
        self.assertIn("<dt>Acknowledgement changed</dt><dd>Yes</dd>", summary)
        # Each account has one semantic entry with both period labels, including absent sides.
        comparison_summary = summary.split('<section id="run-changes"', 1)[1]
        parsed_summary = ReportElements()
        parsed_summary.feed(comparison_summary)
        self.assertEqual(sum(tag == "table" for tag, _ in parsed_summary.elements), 1)
        self.assertEqual(sum(tag == "ul" and attrs.get("class") == "comparison-records"
                             for tag, attrs in parsed_summary.elements), 2)
        self.assertEqual(sum(tag == "h4" for tag, _ in parsed_summary.elements), 8)
        self.assertEqual(comparison_summary.count("<dt>Value in 2024-08-31</dt>"), 4)
        self.assertEqual(comparison_summary.count("<dt>Value in 2024-09-30</dt>"), 4)
        self.assertNotIn("scroll the comparison tables", comparison_summary)
        for change, label in (("NEW", "New"), ("CHANGED", "Changed"), ("RECURRING", "Recurring"),
                              ("NOT_COMPARABLE", "Not comparable")):
            self.assertIn(f'data-change="{change}">{label}</th><td class="amount">1</td><td class="amount">1</td>', summary)
        raw = html.unescape(full.split("<pre>", 1)[1].split("</pre>", 1)[0])
        document = json.loads(raw)
        self.assertEqual(document["findings"], finding_changes)
        self.assertEqual(document["queries"], query_changes)
        self.assertEqual(document["scope_changes"], [attack])
        self.assertEqual(document["changed_inputs"], ["inputs/trial_balance.csv"])
        self.assertTrue(document["acknowledgement_changed"])
        parsed = ReportElements()
        parsed.feed(output)
        identifiers = [attrs["id"] for _, attrs in parsed.elements if "id" in attrs]
        self.assertEqual(len(identifiers), len(set(identifiers)))
        for tag, attrs in parsed.elements:
            if tag == "a":
                href = attrs.get("href")
                assert href is not None
                self.assertIn(href[1:], identifiers)

    def test_empty_comparison_is_distinct_from_no_previous_run(self) -> None:
        output = self.render_comparison({"findings": [], "queries": [], "scope_changes": [],
                                         "review_boundary": "The producer boundary."})
        summary = output.split('<section id="comparison-record"', 1)[0]
        self.assertIn("No finding records occur in either run.", summary)
        self.assertIn("No query records occur in either run.", summary)
        self.assertIn("No scope changes reported by the producer.", summary)
        self.assertIn("<dt>Acknowledgement changed</dt><dd>No</dd>", summary)
        self.assertNotIn("No previous verified run supplied.", output)

    def test_comparison_lists_a_removed_input_separately_from_acknowledgement(self) -> None:
        output = self.render_comparison({"findings": [], "queries": [], "scope_changes": [],
                                         "review_boundary": "The producer boundary."}, removed_input=True)
        record = output.split('<section id="comparison-record"', 1)[1].split("<pre>", 1)[1].split("</pre>", 1)[0]
        change = json.loads(html.unescape(record))
        self.assertEqual(change["changed_inputs"], ["inputs/review-note.json", "inputs/trial_balance.csv"])
        self.assertFalse(change["acknowledgement_changed"])

    def test_changed_producer_is_refused_before_execution(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            executable = bin_dir / ("close-control.exe" if workflow.os.name == "nt" else "close-control")
            executable.write_bytes(b"replacement stub")
            with patch.object(workflow.subprocess, "run") as invoke:
                with self.assertRaisesRegex(ValueError, "hash differs"):
                    workflow.pinned_producer(bin_dir, "close-control", [], manifest_fixture("close-control"))
                invoke.assert_not_called()

    def test_producer_change_during_build_and_display_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            executable = bin_dir / ("close-control.exe" if workflow.os.name == "nt" else "close-control")
            def replace_producer(*args, **kwargs):
                executable.write_bytes(b"changed during invocation")
                return subprocess.CompletedProcess([], 0, stdout=b"", stderr=b"")
            for display in (False, True):
                executable.write_bytes(b"original")
                with self.subTest(display=display), patch.object(workflow.subprocess, "run", side_effect=replace_producer), patch.object(workflow, "producer_manifest", side_effect=lambda *args: manifest_fixture("close-control", executable.read_bytes())):
                    with self.assertRaisesRegex(ValueError, "changed during invocation"):
                        if display:
                            workflow.pinned_producer(bin_dir, "close-control", [], manifest_fixture("close-control"))
                        else:
                            workflow.invoke(bin_dir, "close-control", [], bin_dir, "test")

    def test_unsupported_control_never_receives_inferred_journal_rows(self) -> None:
        # The projection consumes explicit driver identifiers only, never amount matching.
        run_csv = workflow.rows((APP / "samples/sample-review-run.csv").read_bytes())[0]
        finding_csv = workflow.rows((APP / "samples/sample-review-exceptions.csv").read_bytes())
        exceptions = [{"control": r["Control"], "tenant": r["Tenant"], "account_id": r["AccountID"], "account_name": r["Account"], "status": r["Status"], "current_value": r["Current"], "prior_value": r["Prior"], "difference": r["Difference"], "threshold": r["Threshold"], "reason": r["Reason"], "reviewer_action": r["Action"]} for r in finding_csv]
        exceptions[0]["control"] = "unsupported_control"
        case, _ = workflow.sources()
        doc = {"exceptions": exceptions, "client_queries": [], "overall_status": "REVIEW", "prior_report_dates": ["2024-08-31"], "thresholds": {"absolute_variance": "1000.00", "percentage_variance": "10.00%"}, "controls_not_run": ["example omitted control"], "acknowledgement": None}
        snapshot = {"close/close-review-pack.json": workflow.json_bytes(doc), "drivers/variance-drivers.json": workflow.json_bytes({"accounts": []}), "readiness/readiness-pack.json": workflow.json_bytes({"overall_status": "READY", "controls_not_run": []}), "inputs/trial_balance.csv": b"fixture"}
        with patch.object(workflow, "verify", return_value=({"case": case, "period": run_csv["Period"]}, snapshot)):
            projected = workflow.projection(Path("unused"))
        self.assertEqual(workflow.rows(projected["sample-review-evidence.csv"]), [])
        self.assertTrue(all("unavailable" in r["EvidenceState"] for r in workflow.rows(projected["sample-review-exceptions.csv"])))
        self.assertNotIn("coverage_percentage", projected["sample-review-run.csv"].decode())


    def test_export_tree_admits_only_the_readiness_publication_control(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory)
            expected = workflow.fixed_export_inventory()
            for name in expected | {"receipt.json", "receipt.sha256"}:
                (run / name).parent.mkdir(parents=True, exist_ok=True)
                (run / name).write_bytes(b"x")
            control = run / workflow.READY_CONTROL
            control.mkdir()
            (control / "state.json").write_bytes(b"{}")
            workflow.check_export_tree(run, expected)
            (run / "close/.reviewready").mkdir()
            with self.assertRaisesRegex(ValueError, "unlisted"):
                workflow.check_export_tree(run, expected)


if __name__ == "__main__":
    unittest.main()
