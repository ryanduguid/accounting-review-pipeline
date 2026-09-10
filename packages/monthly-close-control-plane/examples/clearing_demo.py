"""Run the fabricated June-August clearing example into a new external directory."""
import argparse
from pathlib import Path

from closecontrol.cli import main


def run(output: Path) -> None:
    examples = Path(__file__).resolve().parent
    periods = [
        ("june", "06", "30", "0", "100", None, None, 2),
        ("july-draft", "07", "31", "100", "25", "june", None, 2),
        ("july-reviewed", "07", "31", "100", "25", "june", "july", 2),
        ("august", "08", "31", "25", "0", "july-reviewed", "august", 0),
    ]
    for name, month, day, opening, closing, previous, decisions, expected in periods:
        source_month = {"06": "june", "07": "july", "08": "august"}[month]
        arguments = ["reconcile", "--transactions", str(examples / f"clearing-{source_month}.csv"),
                     "--tenant", "Demo", "--account-id", "clearing", "--currency", "AUD",
                     "--period-start", f"2026-{month}-01", "--period-end", f"2026-{month}-{day}",
                     "--opening-balance", opening, "--closing-balance", closing,
                     "--output", str(output / name)]
        if previous:
            arguments += ["--opening-items", str(output / previous / "carry-forward.json")]
        if decisions:
            arguments += ["--decisions", str(examples / f"clearing-{decisions}-decisions.csv")]
        result = main(arguments)
        if result != expected:
            raise SystemExit(f"{name}: expected exit {expected}, received {result}")
    print(f"Open {output / 'july-reviewed' / 'review.html'} to inspect the outstanding $25.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    run(parser.parse_args().output)
