"""Cover command failures before decomposing the entry point."""

import io
import os
import sys
import tempfile
import unittest
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

import export_tb

try:
    from .test_export_tb import _ExportCase
except ImportError:
    from test_export_tb import _ExportCase


class CredentialFailures(unittest.TestCase):
    def test_each_missing_credential_stops_before_connections(self):
        for missing in ("XERO_CLIENT_ID", "XERO_CLIENT_SECRET"):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as root:
                previous = os.getcwd()
                os.chdir(root)
                try:
                    values = {"XERO_CLIENT_ID": "fabricated-id", "XERO_CLIENT_SECRET": "fabricated-secret"}
                    del values[missing]
                    with ExitStack() as stack:
                        stack.enter_context(patch.dict(os.environ, values, clear=True))
                        stack.enter_context(patch.object(sys, "argv", [
                            "export_tb.py", "--date", "2026-06-30", "--out", "tb.csv",
                            "--token-file", "token.json",
                        ]))
                        load = stack.enter_context(patch.object(export_tb, "load_dotenv"))
                        connections = stack.enter_context(patch.object(export_tb, "get_connections"))
                        report = stack.enter_context(patch.object(export_tb, "api_get"))
                        stack.enter_context(redirect_stdout(io.StringIO()))
                        with self.assertRaises(SystemExit) as raised:
                            export_tb.main()
                    self.assertEqual(raised.exception.code,
                                     "Set XERO_CLIENT_ID and XERO_CLIENT_SECRET in .env (see .env.example).")
                    load.assert_called_once_with()
                    connections.assert_not_called()
                    report.assert_not_called()
                    self.assertEqual(list(Path(root).iterdir()), [])
                finally:
                    os.chdir(previous)


class ReportFailures(_ExportCase):
    def test_no_connections_stops_before_the_report_call(self):
        with patch.object(export_tb, "validated_connections", return_value=[]), \
                patch.object(export_tb, "flatten_report") as flatten:
            raised, stdout, data = self.run_export([], tenant_name="Demo Company AU")
        self.assertEqual(raised.code, "No Xero organisations authorised for this app - run auth.py again.")
        self.assertEqual(stdout, "")
        self.assertIsNone(data)
        flatten.assert_not_called()

    def test_empty_reports_preserves_the_tenant_message_and_writes_nothing(self):
        raised, stdout, data = self.run_export([], payload={"Reports": []}, tenant_name="Demo Company AU")
        self.assertEqual(raised.code, "Empty Reports payload - check the date parameter and API scopes.")
        self.assertEqual(stdout, "Tenant: Demo Company AU\n")
        self.assertIsNone(data)
        self.assertFalse(Path(self.work_dir, "tb.csv.manifest.json").exists())

    def test_final_output_rejection_stops_before_building_rows(self):
        with patch.object(export_tb, "output_path", side_effect=["tb.csv", ValueError("invalid selected destination")]), \
                patch.object(export_tb, "build_rows") as build, redirect_stderr(io.StringIO()) as stderr:
            raised, stdout, data = self.run_export([
                ("Bank (090)", "1.00", "", "1.00", ""),
                ("Payables (800)", "", "1.00", "", "1.00"),
            ], tenant_name="Demo Company AU")
        self.assertEqual(raised.code, 2)
        self.assertEqual(stdout, "Tenant: Demo Company AU\n")
        self.assertIn("error: invalid selected destination\n", stderr.getvalue())
        self.assertIsNone(data)
        build.assert_not_called()
