from __future__ import annotations

import argparse
import csv
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .engine import review_close
from .errors import ControlInputError
from .reconciliation import reconcile
from .reconciliation_report import write_reconciliation
from .report import (
    PACK_FILE_NAMES as _PACK_FILE_NAMES,
)
from .report import (
    require_output_outside_repository,
    write_review_pack,
)
from .viewer import render_review_sheet


def _non_negative_decimal(value: str) -> Decimal:
    try:
        result = Decimal(value)
    except InvalidOperation as exc:
        raise argparse.ArgumentTypeError(f"not a decimal: {value!r}") from exc
    if not result.is_finite() or result < 0:
        raise argparse.ArgumentTypeError("must be a finite non-negative decimal")
    return result


def _add_close_arguments(command: argparse.ArgumentParser) -> None:
    """Add the one validated input contract shared by close entry points."""
    command.add_argument("--current", required=True, type=Path, help="current-period canonical trial-balance CSV")
    command.add_argument("--prior", required=True, type=Path, help="prior-period canonical trial-balance CSV")
    command.add_argument("--mapping", type=Path, help="optional AccountID,ReviewGroup mapping CSV")
    command.add_argument("--subledger", type=Path, help="optional Tenant,AccountID,SubledgerBalance CSV")
    command.add_argument("--review-note", type=Path, help="optional human acknowledgement JSON")
    command.add_argument("--output", required=True, type=Path, help="directory for the generated review pack")
    command.add_argument("--absolute-threshold", type=_non_negative_decimal, default=Decimal("1000"), help="minimum absolute YTD variance for review")
    command.add_argument("--percentage-threshold", type=_non_negative_decimal, default=Decimal("0.10"), help="minimum proportional YTD variance for review, e.g. 0.10")
    command.add_argument("--reconciliation-tolerance", type=_non_negative_decimal, default=Decimal("0.01"), help="maximum permitted GL/subledger difference")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Create a review-first monthly close control pack from validated trial-balance exports.")
    commands = parser.add_subparsers(dest="command", required=True)
    review = commands.add_parser("review", help="run integrity, variance, mapping, and optional reconciliation controls")
    _add_close_arguments(review)
    workbench = commands.add_parser("workbench", help="run the local close-review workbench")
    _add_close_arguments(workbench)
    view = commands.add_parser("view", help="display an existing review pack after verifying its four files agree")
    view.add_argument("--pack-dir", required=True, type=Path, help="directory holding close-review-pack.json, close-summary.md, exceptions.csv and client-queries.csv")
    clearing = commands.add_parser("reconcile", help="review clearing-account transactions and carry outstanding items forward")
    clearing.add_argument("--transactions", required=True, type=Path, help="mapped transaction CSV; see docs/clearing-reconciliation.md")
    for flag in ("tenant", "account-id", "currency", "period-start", "period-end", "opening-balance", "closing-balance"):
        clearing.add_argument(f"--{flag}", required=True)
    clearing.add_argument("--opening-items", type=Path, help="previous period's carry-forward.json")
    clearing.add_argument("--decisions", type=Path, help="reviewed Group,TransactionID,Decision,Note CSV")
    clearing.add_argument("--output", required=True, type=Path, help="new directory outside version control")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        # argparse exits with 2 on usage errors, but this tool's exit contract
        # reserves 2 for a pack needing review; remap usage errors to 1.
        # --help and -h exit with 0 and must keep doing so, so re-raise
        # anything that is not a usage error.
        if exc.code == 2:
            return 1
        raise
    if args.command not in {"review", "workbench", "view", "reconcile"}:  # pragma: no cover - argparse validates command choices.
        parser.error("unknown command")
    if args.command == "reconcile":
        try:
            destination = require_output_outside_repository(args.output)
            if destination.exists():
                raise ControlInputError("Output already exists; use a new directory for each run.")
            reconciliation = reconcile(args.transactions, tenant=args.tenant, account_id=args.account_id,
                             currency=args.currency, period_start=args.period_start,
                             period_end=args.period_end, opening_balance=args.opening_balance,
                             closing_balance=args.closing_balance, opening_items=args.opening_items,
                             decisions=args.decisions)
            review_path = write_reconciliation(reconciliation, destination)
        except (ControlInputError, OSError, ValueError, csv.Error) as exc:
            print(f"close-control reconcile: {exc}", file=sys.stderr)
            return 1
        print(f"close-control reconcile: {reconciliation['status']}; {len(reconciliation['outstanding'])} outstanding items")
        print(f"  Review: {review_path}")
        return 0 if reconciliation["status"] == "PASS" else 2
    if args.command == "view":
        # The viewer reads only. It never writes, renames or deletes, so the
        # source/destination collision guard below does not apply to it.
        try:
            sheet, _ = render_review_sheet(args.pack_dir)
        except ControlInputError as exc:
            print(f"close-control view: verification failed: {exc}", file=sys.stderr)
            return 1
        print(sheet)
        print(
            "close-control view: display is a review aid; it does not approve "
            "a close or change any computed status."
        )
        return 0
    # Both output guards run before any client file is opened. write_review_pack
    # repeats this one so a library caller cannot get past it, but a reviewer who
    # mistyped --output should hear about it before the run reads a trial
    # balance, not after.
    try:
        require_output_outside_repository(args.output)
    except ControlInputError as exc:
        print(f"close-control: output error: {exc}", file=sys.stderr)
        return 1
    # write_review_pack replaces its 4 destinations and deletes what it
    # parked aside. If a source file IS one of those destinations, that source
    # is destroyed and the pack still records a source_sha256 for it, so the
    # provenance chain points at evidence that no longer exists. Refuse before
    # anything is read or written. resolve() so case, "." segments and symlinks
    # normalise to one key; the destinations need not exist yet.
    destinations = {(args.output / name).resolve() for name in _PACK_FILE_NAMES}
    for flag, source in (
        ("--current", args.current),
        ("--prior", args.prior),
        ("--mapping", args.mapping),
        ("--subledger", args.subledger),
        ("--review-note", args.review_note),
    ):
        if source is not None and source.resolve() in destinations:
            print(
                f"close-control: output error: {flag} {source} is inside --output and "
                f"shares a generated pack file name; the run would destroy it.",
                file=sys.stderr,
            )
            return 1
    try:
        pack = review_close(
            current_path=args.current,
            prior_path=args.prior,
            mapping_path=args.mapping,
            subledger_path=args.subledger,
            acknowledgement_path=args.review_note,
            absolute_threshold=args.absolute_threshold,
            percentage_threshold=args.percentage_threshold,
            reconciliation_tolerance=args.reconciliation_tolerance,
        )
    except (ControlInputError, ValueError) as exc:
        print(f"close-control: input error: {exc}", file=sys.stderr)
        return 1
    try:
        outputs = write_review_pack(pack, args.output)
    except (OSError, ValueError) as exc:
        # An unusable --output is a caller error, not a crash: report it on the
        # same failure path as a malformed input instead of a raw traceback.
        # ValueError covers a render that cannot be encoded, which is not an
        # OSError and used to escape as a traceback.
        print(f"close-control: output error: {exc}", file=sys.stderr)
        return 1
    prefix = "close-control workbench" if args.command == "workbench" else "close-control"
    print(
        f"{prefix}: {pack.status}; {len(pack.exceptions)} exception(s); "
        f"{len(pack.client_queries)} client query(ies) drafted"
    )
    if pack.status == "PASS" and not pack.exceptions:
        print(f"{prefix}: Nothing interesting happens.")
    for name, path in outputs.items():
        print(f"  {name}: {path}")
    if args.command == "workbench":
        print("Review close-summary.md, exceptions.csv, client-queries.csv, and close-review-pack.json.")
        print("This pack records review evidence only; it does not approve or close a period.")
        print("client-queries.csv holds draft questions; nothing has been sent to a client.")
    return 0 if pack.status == "PASS" else 2


if __name__ == "__main__":
    sys.exit(main())
