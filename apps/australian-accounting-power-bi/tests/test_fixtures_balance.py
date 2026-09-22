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
from decimal import Decimal
from pathlib import Path
from unittest import mock

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
    def test_balance_oracle_rejects_a_cent_below_float_precision(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "sample-general-ledger.csv").write_text(
                "JournalID,Debit,Credit,Amount\n"
                "JNL1,10000000000000000.01,10000000000000000.00,0.01\n",
                encoding="utf-8",
            )
            with mock.patch.dict(globals(), SAMPLES_DIR=root):
                with self.assertRaisesRegex(AssertionError, "unbalanced"):
                    self.test_general_ledger_journals_strictly_balanced()

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
            total_debit = sum((Decimal(line["Debit"]) for line in lines), Decimal(0))
            total_credit = sum((Decimal(line["Credit"]) for line in lines), Decimal(0))
            net_amount = sum((Decimal(line["Amount"]) for line in lines), Decimal(0))

            self.assertEqual(
                total_debit,
                total_credit,
                f"Journal {jid} is unbalanced: Debits ({total_debit}) != Credits ({total_credit})",
            )
            self.assertEqual(
                net_amount,
                Decimal(0),
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
        ic_amounts: list[Decimal] = []

        with open(gl_path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                if row["IsIntercompany"] == "TRUE":
                    ic_amounts.append(Decimal(row["Amount"]))

        self.assertGreater(len(ic_amounts), 0, "GL fixture must contain intercompany entries")
        total_ic_net = sum(ic_amounts, Decimal(0))
        self.assertEqual(
            total_ic_net,
            Decimal(0),
            f"Intercompany aggregate net movement does not eliminate to zero: {total_ic_net}",
        )


class TestFixtureRegeneration(unittest.TestCase):
    def test_regenerating_reproduces_the_committed_fixtures_byte_for_byte(self) -> None:
        """README calls samples/ the deterministic output of tools/generate_fixtures.py.

        The generator wrote CRLF while the committed CSVs are LF, so anyone who ran it
        rewrote all 6 files end to end: 2,401 changed lines that are identical once
        newlines are normalised. Churn on that scale hides any real fixture edit inside
        it and leaves the determinism claim unreviewable.
        """
        spec = importlib.util.spec_from_file_location("generate_fixtures", GENERATOR_FILE)
        self.assertIsNotNone(spec, "generate_fixtures.py must be importable")
        generator = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
        spec.loader.exec_module(generator)  # type: ignore[union-attr]

        with tempfile.TemporaryDirectory() as scratch:
            setattr(generator, "SAMPLES_DIR", Path(scratch))
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
