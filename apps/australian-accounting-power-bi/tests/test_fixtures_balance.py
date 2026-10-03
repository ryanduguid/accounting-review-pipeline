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
        """Each bilateral pair balances separately for balance sheet and P&L legs."""
        with (SAMPLES_DIR / "sample-chart-of-accounts.csv").open(encoding="utf-8") as handle:
            classes = {row["AccountCode"]: row["Class"] for row in csv.DictReader(handle)}
        with (SAMPLES_DIR / "sample-general-ledger.csv").open(encoding="utf-8") as handle:
            rows = [row for row in csv.DictReader(handle) if row["IsIntercompany"] == "TRUE"]
        self.assertTrue(rows)
        groups: dict[tuple[str, str, str], list[dict[str, str]]] = {}
        for row in rows:
            pair = "/".join(sorted((row["EntityID"], row["IntercompanyEntityID"])))
            leg = "PL" if classes[row["AccountCode"]] in {"Revenue", "Expense"} else "BS"
            groups.setdefault((row["PostingDate"], pair, leg), []).append(row)
        for key, lines in groups.items():
            self.assertEqual(len({line["EntityID"] for line in lines}), 2, f"Unmatched pair {key}")
            self.assertEqual(sum((Decimal(line["Amount"]) for line in lines), Decimal(0)), 0, f"Unmatched pair {key}")

    def test_pair_oracle_rejects_a_missing_balanced_counterparty(self) -> None:
        with (SAMPLES_DIR / "sample-general-ledger.csv").open(encoding="utf-8") as handle:
            reader = csv.DictReader(handle)
            fields = reader.fieldnames
            rows = list(reader)
        target = next(row["JournalID"] for row in rows if row["EntityID"] == "ENT002" and row["IsIntercompany"] == "TRUE")
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "sample-chart-of-accounts.csv").write_bytes((SAMPLES_DIR / "sample-chart-of-accounts.csv").read_bytes())
            with (root / "sample-general-ledger.csv").open("w", newline="", encoding="utf-8") as handle:
                writer = csv.DictWriter(handle, fieldnames=fields or [])
                writer.writeheader()
                writer.writerows(row for row in rows if row["JournalID"] != target)
            with mock.patch.dict(globals(), SAMPLES_DIR=root):
                self.test_general_ledger_journals_strictly_balanced()
                with self.assertRaisesRegex(AssertionError, "Unmatched pair"):
                    self.test_intercompany_transactions_match_across_group()


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
