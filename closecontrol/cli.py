from __future__ import annotations

import argparse
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .engine import review_close
from .errors import ControlInputError
from .report import PACK_FILE_NAMES as _PACK_FILE_NAMES, write_review_pack


def _non_negative_decimal(value: str) -> Decimal:
    try:
        result = Decimal(value)
    except InvalidOperation as exc:
        raise argparse.ArgumentTypeError(f"not a decimal: {value!r}") from exc
    if not result.is_finite() or result < 0:
        raise argparse.ArgumentTypeError("must be a finite non-negative decimal")
    return result


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Create a review-first monthly close control pack from validated trial-balance exports.")
    commands = parser.add_subparsers(dest="command", required=True)
    review = commands.add_parser("review", help="run integrity, variance, mapping, and optional reconciliation controls")
    review.add_argument("--current", required=True, type=Path, help="current-period canonical trial-balance CSV")
    review.add_argument("--prior", required=True, type=Path, help="prior-period canonical trial-balance CSV")
    review.add_argument("--mapping", type=Path, help="optional AccountID,ReviewGroup mapping CSV")
    review.add_argument("--subledger", type=Path, help="optional Tenant,AccountID,SubledgerBalance CSV")
    review.add_argument("--review-note", type=Path, help="optional human acknowledgement JSON")
    review.add_argument("--output", required=True, type=Path, help="directory for the generated review pack")
    review.add_argument("--absolute-threshold", type=_non_negative_decimal, default=Decimal("1000"), help="minimum absolute YTD variance for review")
    review.add_argument("--percentage-threshold", type=_non_negative_decimal, default=Decimal("0.10"), help="minimum proportional YTD variance for review, e.g. 0.10")
    review.add_argument("--reconciliation-tolerance", type=_non_negative_decimal, default=Decimal("0.01"), help="maximum permitted GL/subledger difference")
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
    if args.command != "review":  # pragma: no cover - argparse currently has one subcommand.
        parser.error("unknown command")
    # write_review_pack replaces its three destinations and deletes what it
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
    print(f"close-control: {pack.status}; {len(pack.exceptions)} exception(s)")
    for name, path in outputs.items():
        print(f"  {name}: {path}")
    return 0 if pack.status == "PASS" else 2


if __name__ == "__main__":
    sys.exit(main())
