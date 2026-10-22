"""Joined offline conformance for the data-only Xero trial-balance contract.

The exporter-owned integrity runner and the offline review implementations are
exercised against every root fixture. The Python review packages are imported
from their directories; nothing here installs them, reaches a network, or reads
a credential. The Rust ledger-review gate runs the same accept/reject boundary,
identity and account-code checks in its own cargo test
(test_gateway_satisfies_the_pinned_exporter_contract), which
joined-conformance.yml runs beside this file. The exporter integrity runner is Rust.
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
from tests.native_review_ready import probe


ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "contracts" / "xero-trial-balance-v1"
EXPORTER_MANIFEST = ROOT / "packages" / "xero-trial-balance-export" / "Cargo.toml"
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
        # The fixture directory too, or a new CSV joins the tree with no digest and
        # no scenario, and never reaches the exporter or its consumers.
        fixture_names = {path.name for path in (CONTRACT / "fixtures").glob("*.csv")}
        self.assertEqual(fixture_names, set(FIXTURE_DIGESTS))
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
                    ["cargo", "run", "--quiet", "--locked", "--features", "tools", "--bin", "xero-tb-evaluate", "--manifest-path", str(EXPORTER_MANIFEST), "--", str(fixture)],
                    cwd=EXPORTER_MANIFEST.parent,
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, scenarios[name]["exit_code"], result.stdout + result.stderr)
                for marker in scenarios[name]["output_contains"]:
                    self.assertIn(marker, result.stdout + result.stderr)

    def test_all_offline_review_consumers_keep_the_same_accept_reject_boundary(self) -> None:
        sys.path[:0] = [
            str(ROOT / "packages" / "monthly-close-control-plane"),
        ]
        from closecontrol.engine import _integrity_exceptions
        from closecontrol.loader import load_canonical_tb as load_monthly

        expected = {
            "passing.csv": (True, Decimal("0"), Decimal("0")),
            "failing_movement.csv": (False, Decimal("0.01"), Decimal("0")),
            "failing_ytd.csv": (False, Decimal("0"), Decimal("0.01")),
        }
        for name, (accept, movement, ytd) in expected.items():
            with self.subTest(name=name):
                fixture = CONTRACT / "fixtures" / name

                # Readiness gate: exact Decimal movement and YTD differences, no tolerance.
                readiness = probe({"op": "tb_integrity", "path": str(fixture)})["ok"]
                self.assertEqual((Decimal(readiness["movement"]), Decimal(readiness["ytd"])), (movement, ytd))
                self.assertEqual(tuple((row["tenant"], row["account_id"]) for row in readiness["rows"]), IDENTITY)
                self.assertEqual(tuple(row["account_code"] for row in readiness["rows"]), ACCOUNT_CODES)

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


if __name__ == "__main__":
    unittest.main()
