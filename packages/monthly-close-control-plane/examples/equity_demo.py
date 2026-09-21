"""Run the fabricated equity case and verify the resulting pack."""
import argparse
import hashlib
import json
from pathlib import Path

from closecontrol.engine import review_close
from closecontrol.report import require_output_outside_repository, write_review_pack
from closecontrol.viewer import verify_pack

HEADER = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit"


def run(output: Path) -> None:
    output = require_output_outside_repository(output)
    output.mkdir(parents=True, exist_ok=False)
    paths = {}
    for label, when, amount in (("prior", "2026-08-31", "100000"),
                                ("current", "2026-09-30", "129750")):
        path = output / f"{label}.csv"
        path.write_text(HEADER + "\n"
                        + f"{when},Synthetic,Assets,A,Bank,090,{amount},0,{amount},0\n"
                        + f"{when},Synthetic,Equity,E,Capital,300,0,{amount},0,{amount}\n",
                        encoding="utf-8")
        paths[label] = path
    schedule = {
        "schema_version": 1, "tenant": "Synthetic", "opening_date": "2026-08-31",
        "closing_date": "2026-09-30", "currency": "AUD", "basis": "ledger_equity",
        "prior_source_sha256": hashlib.sha256(paths["prior"].read_bytes()).hexdigest(),
        "current_source_sha256": hashlib.sha256(paths["current"].read_bytes()).hexdigest(),
        "equity_account_ids": ["E"], "complete": True,
        "movements": [
            {"movement_id": key, "date": "2026-09-30", "account_id": "E", "debit": debit,
             "credit": credit, "description": description, "evidence_reference": "Synthetic " + key}
            for key, debit, credit, description in (
                ("contribution", "0", "25000", "Capital contribution"),
                ("withdrawal", "10000", "0", "Owner withdrawal"),
                ("transfer", "0", "15000", "Evidenced profit transfer"),
            )
        ],
    }
    schedule_path = output / "schedule.json"
    schedule_path.write_text(json.dumps(schedule, indent=2) + "\n", encoding="utf-8")
    pack = review_close(current_path=paths["current"], prior_path=paths["prior"],
                        equity_schedule_path=schedule_path, equity_currency="AUD",
                        equity_currency_evidence="Fabricated report metadata: AUD")
    assert pack.equity_reconciliation["total"]["unexplained"] == "-250"
    write_review_pack(pack, output / "pack")
    verify_pack(output / "pack")
    print(f"Equity example verified: -250 unexplained. Pack: {output / 'pack'}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    run(parser.parse_args().output)

