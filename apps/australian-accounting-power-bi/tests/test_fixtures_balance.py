"""
test_fixtures_balance.py - Verifies double-entry accounting integrity of sample fixtures.
"""

from __future__ import annotations

import contextlib
import csv
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
SAMPLES_DIR = BASE_DIR / "samples"
GENERATOR_FILE = BASE_DIR / "tools" / "generate_fixtures.py"
FIXTURE_FILES = [
    "sample-entities.csv",
    "sample-chart-of-accounts.csv",
    "sample-general-ledger.csv",
    "sample-budgets.csv",
    "sample-payroll-super.csv",
    "sample-ato-benchmarks.csv",
]


class TestFixturesBalance(unittest.TestCase):
    def test_sample_files_exist(self) -> None:
        for fname in FIXTURE_FILES:
            fpath = SAMPLES_DIR / fname
            self.assertTrue(fpath.is_file(), f"Missing required fixture: {fname}")

    def test_general_ledger_journals_strictly_balanced(self) -> None:
        """Every individual journal entry in the GL must have sum(Debits) == sum(Credits)."""
        gl_path = SAMPLES_DIR / "sample-general-ledger.csv"
        journals: dict[str, list[dict[str, str]]] = {}

        with open(gl_path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                jid = row["JournalID"]
                journals.setdefault(jid, []).append(row)

        self.assertGreater(len(journals), 0, "GL fixture must contain journals")

        for jid, lines in journals.items():
            total_debit = round(sum(float(l["Debit"]) for l in lines), 2)
            total_credit = round(sum(float(l["Credit"]) for l in lines), 2)
            net_amount = round(sum(float(l["Amount"]) for l in lines), 2)

            self.assertEqual(
                total_debit,
                total_credit,
                f"Journal {jid} is unbalanced: Debits ({total_debit}) != Credits ({total_credit})",
            )
            self.assertEqual(
                net_amount,
                0.0,
                f"Journal {jid} net amount is non-zero: {net_amount}",
            )

    def test_chart_of_accounts_uniqueness(self) -> None:
        """All account codes must be unique and properly categorised."""
        coa_path = SAMPLES_DIR / "sample-chart-of-accounts.csv"
        codes: set[str] = set()

        with open(coa_path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                code = row["AccountCode"]
                self.assertNotIn(code, codes, f"Duplicate account code: {code}")
                codes.add(code)
                self.assertIn(row["Class"], ["Asset", "Liability", "Equity", "Revenue", "Expense"])
                self.assertIn(row["NormalBalance"], ["Debit", "Credit"])

    def test_intercompany_transactions_match_across_group(self) -> None:
        """Intercompany transactions must balance to zero when aggregated across the group."""
        gl_path = SAMPLES_DIR / "sample-general-ledger.csv"
        ic_amounts: list[float] = []

        with open(gl_path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                if row["IsIntercompany"] == "TRUE":
                    ic_amounts.append(float(row["Amount"]))

        self.assertGreater(len(ic_amounts), 0, "GL fixture must contain intercompany entries")
        total_ic_net = round(sum(ic_amounts), 2)
        self.assertEqual(
            total_ic_net,
            0.0,
            f"Intercompany aggregate net movement does not eliminate to zero: {total_ic_net}",
        )


class TestFixtureRegeneration(unittest.TestCase):
    def test_regenerating_reproduces_the_committed_fixtures_byte_for_byte(self) -> None:
        """README calls samples/ the deterministic output of tools/generate_fixtures.py.

        The generator wrote CRLF while the committed CSVs are LF, so anyone who ran it
        rewrote all six files end to end: 2,401 changed lines that are identical once
        newlines are normalised. Churn on that scale hides any real fixture edit inside
        it and leaves the determinism claim unreviewable.
        """
        spec = importlib.util.spec_from_file_location("generate_fixtures", GENERATOR_FILE)
        self.assertIsNotNone(spec, "generate_fixtures.py must be importable")
        generator = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
        spec.loader.exec_module(generator)  # type: ignore[union-attr]

        with tempfile.TemporaryDirectory() as scratch:
            generator.SAMPLES_DIR = Path(scratch)
            with contextlib.redirect_stdout(io.StringIO()):
                generator.generate_fixtures()

            differing = [
                fname
                for fname in FIXTURE_FILES
                if (Path(scratch) / fname).read_bytes() != (SAMPLES_DIR / fname).read_bytes()
            ]

        self.assertEqual(
            differing,
            [],
            "Regenerated fixtures differ from the committed bytes: " + ", ".join(differing),
        )


if __name__ == "__main__":
    unittest.main()
