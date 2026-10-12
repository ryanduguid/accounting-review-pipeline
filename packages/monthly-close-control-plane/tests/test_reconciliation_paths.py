"""Reconciliation gates on decisions, configuration, CSV parsing, size and output reuse."""
import csv

import pytest
from closecontrol.errors import ControlInputError

from test_reconciliation import compute, decision_file, write_transactions

ROWS = ["s1,2026-07-03,Batch,Sale,60,0", "p1,2026-07-04,Batch,Settlement,0,60"]


def test_short_decisions_row_is_refused(tmp_path):
    source = write_transactions(tmp_path / "transactions.csv", ROWS)
    decisions = decision_file(tmp_path, ["batch,s1,accept"])
    with pytest.raises(ControlInputError, match="Every decisions row must contain all four fields"):
        compute(source, decisions=decisions)


def test_unknown_decision_word_is_refused(tmp_path):
    source = write_transactions(tmp_path / "transactions.csv", ROWS)
    decisions = decision_file(tmp_path, ["batch,s1,approve,Checked", "batch,p1,approve,Checked"])
    with pytest.raises(ControlInputError, match="Decision must be blank, accept or reject"):
        compute(source, decisions=decisions)


def test_currency_must_be_an_uppercase_code(tmp_path):
    source = write_transactions(tmp_path / "transactions.csv", ROWS)
    with pytest.raises(ControlInputError, match="Currency must be a three-letter uppercase code"):
        compute(source, currency="aud")


def test_csv_parser_error_is_an_input_error(tmp_path):
    source = write_transactions(tmp_path / "transactions.csv",
                                [f"s1,2026-07-03,{'x' * (csv.field_size_limit() + 1)},Sale,60,0"])
    with pytest.raises(ControlInputError, match="Transactions is not valid CSV: field larger than field limit"):
        compute(source)


def test_more_than_100000_items_is_refused(tmp_path):
    source = write_transactions(tmp_path / "transactions.csv",
                                [f"t{index},2026-07-01,R,Sale,1,0" for index in range(100001)])
    with pytest.raises(ControlInputError, match="at most 100,000 outstanding and current items"):
        compute(source)


def test_existing_output_directory_is_not_overwritten(tmp_path):
    from closecontrol.reconciliation_report import write_reconciliation

    pack = compute(write_transactions(tmp_path / "transactions.csv", ROWS))
    output = tmp_path / "pack"
    output.mkdir()
    with pytest.raises(ValueError, match="Output already exists"):
        write_reconciliation(pack, output)
    assert list(output.iterdir()) == []  # nosec B101: pytest outcome assertion
