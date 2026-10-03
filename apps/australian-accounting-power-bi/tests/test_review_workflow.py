"""Regression checks for the synthetic journal boundary and review consumers."""
from __future__ import annotations

import html
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from contextlib import ExitStack
from decimal import Decimal
from html.parser import HTMLParser
from pathlib import Path
from unittest.mock import MagicMock, patch

APP = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("review_workflow", APP / "tools/review_workflow.py")
assert spec and spec.loader
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)


class ReportElements(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.elements: list[tuple[str, dict[str, str | None]]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.elements.append((tag, dict(attrs)))


class ReviewWorkflowTests(unittest.TestCase):
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
        receipt = {"files": {attack: attack}, "invocations": [{"command": command, "executable_sha256": "0" * 64} for command in ("close-control", "review-ready")]}
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
        receipt = {"files": {}, "invocations": [{"command": command, "executable_sha256": "0" * 64} for command in ("close-control", "review-ready")]}
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
            receipt = {"invocations": [{"command": "close-control", "executable_sha256": workflow.digest(executable.read_bytes())}]}
            with patch.object(workflow, "verify", return_value=(receipt, {})), patch.object(workflow.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
                with self.assertRaisesRegex(ValueError, "Producer verification refused"):
                    workflow.render_html(Path("unused"), bin_dir)

    def render_comparison(self, comparison: dict, *, acknowledgement: bool = False) -> str:
        projected = {name: (APP / "samples" / name).read_bytes() for name in
                     ("sample-review-run.csv", "sample-review-exceptions.csv", "sample-review-evidence.csv")}
        previous, current = Path("previous"), Path("current")
        invocations = [{"command": command, "executable_sha256": "0" * 64}
                       for command in ("close-control", "review-ready")]
        def verified(run):
            return ({"case": "same fabricated case", "period": "2024-08-31" if run == previous else "2024-09-30",
                     "files": {"inputs/trial_balance.csv": "old" if run == previous else "new"},
                     "invocations": invocations},
                    {"close/close-review-pack.json": workflow.json_bytes({"acknowledgement":
                     "fabricated note" if run == current and acknowledgement else None})})
        with patch.object(workflow, "verify", side_effect=verified), patch.object(workflow, "projection", return_value=projected), patch.object(workflow, "pinned_producer", return_value=workflow.json_bytes(comparison)):
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
        self.assertEqual(sum(tag == "h4" for tag, _ in parsed_summary.elements), 10)
        self.assertEqual(comparison_summary.count("<dt>Value in 2024-08-31</dt>"), 5)
        self.assertEqual(comparison_summary.count("<dt>Value in 2024-09-30</dt>"), 5)
        self.assertNotIn("scroll the comparison tables", comparison_summary)
        for change, label in (("NEW", "New"), ("CHANGED", "Changed"), ("RECURRING", "Recurring"),
                              ("NOT_RAISED", "Not raised"), ("NOT_COMPARABLE", "Not comparable")):
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

    def test_changed_producer_is_refused_before_execution(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            executable = bin_dir / ("close-control.exe" if workflow.os.name == "nt" else "close-control")
            executable.write_bytes(b"replacement stub")
            with patch.object(workflow.subprocess, "run") as invoke:
                with self.assertRaisesRegex(ValueError, "hash differs"):
                    workflow.pinned_producer(bin_dir, "close-control", [], workflow.digest(b"original"))
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
                with self.subTest(display=display), patch.object(workflow.subprocess, "run", side_effect=replace_producer):
                    with self.assertRaisesRegex(ValueError, "changed during invocation"):
                        if display:
                            workflow.pinned_producer(bin_dir, "close-control", [], workflow.digest(b"original"))
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


if __name__ == "__main__":
    unittest.main()
