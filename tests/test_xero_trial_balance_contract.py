"""Joined offline conformance for the data-only Xero trial-balance contract.

The exporter-owned integrity runner and the three offline review implementations
are exercised against every root fixture. The review packages are imported from
their directories; nothing here installs them, reaches a network, or reads a
credential. The only runtime dependency is the exporter's hash-locked lock file.
"""

from __future__ import annotations

import csv
import hashlib
import json
import subprocess
import sys
import unittest
from decimal import Decimal
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "contracts" / "xero-trial-balance-v1"
EXPORTER_RUNNER = (
    ROOT
    / "packages"
    / "xero-trial-balance-export"
    / "evaluation"
    / "xero_tb_integrity"
    / "run.py"
)
HEADER = (
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
)
FIXTURE_DIGESTS = {
    "passing.csv": "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
    "failing_movement.csv": "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
    "failing_ytd.csv": "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
}
TENANT = "Catherby Fisheries Pty Ltd"
# Tenant plus the stable Xero AccountID is the control identity in every consumer.
IDENTITY = (
    (TENANT, "00000000-0000-0000-0000-000000000001"),
    (TENANT, "00000000-0000-0000-0000-000000000002"),
)
# AccountCode is presentation metadata; the leading zero must survive as text.
ACCOUNT_CODES = ("090", "800")


class CanonicalContractTest(unittest.TestCase):
    def test_exporter_owned_bytes_and_outcomes_are_the_root_contract(self) -> None:
        contract = json.loads((CONTRACT / "expected_results.json").read_text(encoding="utf-8"))
        self.assertEqual(contract["corpus_id"], "xero-tb-csv.v1")
        self.assertEqual(tuple(contract["canonical_columns"]), HEADER)
        self.assertEqual((CONTRACT / "schema.csv").read_bytes(), (",".join(HEADER) + "\n").encode())

        scenarios = {item["fixture"]: item for item in contract["scenarios"]}
        self.assertEqual(set(scenarios), set(FIXTURE_DIGESTS))
        for name, digest in FIXTURE_DIGESTS.items():
            with self.subTest(name=name):
                fixture = CONTRACT / "fixtures" / name
                content = fixture.read_bytes()
                self.assertNotIn(b"\r\n", content)
                self.assertEqual(hashlib.sha256(content).hexdigest(), digest)
                with fixture.open(encoding="utf-8-sig", newline="") as source:
                    self.assertEqual(tuple(next(csv.reader(source))), HEADER)
                result = subprocess.run(
                    [sys.executable, str(EXPORTER_RUNNER), str(fixture)],
                    cwd=ROOT,
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, scenarios[name]["exit_code"], result.stdout + result.stderr)
                for marker in scenarios[name]["output_contains"]:
                    self.assertIn(marker, result.stdout + result.stderr)

    def test_all_offline_review_consumers_keep_the_same_accept_reject_boundary(self) -> None:
        sys.path[:0] = [
            str(ROOT / "packages" / "review-ready-gate"),
            str(ROOT / "packages" / "monthly-close-control-plane"),
            str(ROOT / "packages" / "elizabeth-anne-alexander"),
        ]
        from closecontrol.engine import _integrity_exceptions
        from closecontrol.loader import load_canonical_tb as load_monthly
        from elizabeth_anne_alexander.errors import GatewayError
        from elizabeth_anne_alexander.gateway import _load_tb
        from reviewready.engine import _tb_balanced
        from reviewready.loader import SourceSnapshot, load_canonical_tb as load_readiness

        expected = {
            "passing.csv": (True, Decimal("0"), Decimal("0")),
            "failing_movement.csv": (False, Decimal("0.01"), Decimal("0")),
            "failing_ytd.csv": (False, Decimal("0"), Decimal("0.01")),
        }
        for name, (accept, movement, ytd) in expected.items():
            with self.subTest(name=name):
                fixture = CONTRACT / "fixtures" / name

                # Readiness gate: exact Decimal movement and YTD differences, no tolerance.
                readiness_rows = load_readiness(
                    SourceSnapshot.capture(fixture, label="Trial-balance file")
                )
                self.assertEqual(_tb_balanced(readiness_rows), (movement, ytd))
                self.assertEqual(tuple((row.tenant, row.account_id) for row in readiness_rows), IDENTITY)
                self.assertEqual(tuple(row.account_code for row in readiness_rows), ACCOUNT_CODES)

                # Monthly close: a blocked integrity exception carries the exact 0.01 difference.
                monthly_rows = load_monthly(fixture)
                monthly_findings = _integrity_exceptions(monthly_rows, "current")
                self.assertEqual(monthly_findings == [], accept)
                self.assertEqual(
                    [(item.status, item.difference) for item in monthly_findings],
                    [] if accept else [("BLOCKED", movement + ytd)],
                )
                self.assertEqual(tuple(row.key for row in monthly_rows), IDENTITY)
                self.assertEqual(tuple(row.account_code for row in monthly_rows), ACCOUNT_CODES)

                # Ledger review: the gateway loader accepts only a balanced file.
                if accept:
                    ledger_rows = _load_tb(fixture)
                    self.assertEqual(tuple((row.tenant, row.account_id) for row in ledger_rows), IDENTITY)
                    self.assertEqual(tuple(row.account_code for row in ledger_rows), ACCOUNT_CODES)
                else:
                    with self.assertRaises(GatewayError):
                        _load_tb(fixture)


if __name__ == "__main__":
    unittest.main()
