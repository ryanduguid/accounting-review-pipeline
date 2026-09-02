from __future__ import annotations

import argparse
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .engine import review_pack
from .errors import GateInputError
from .profiles import PROFILE_NAMES
from .report import PACK_FILE_NAMES, write_review_pack
from .viewer import render_review_sheet


def _non_negative_decimal(value: str) -> Decimal:
    try:
        result = Decimal(value)
    except InvalidOperation as exc:
        raise argparse.ArgumentTypeError(f"not a decimal: {value!r}") from exc
    if not result.is_finite() or result < 0:
        raise argparse.ArgumentTypeError("must be a finite non-negative decimal")
    return result


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Gate a workpaper pack before it reaches manager review. "
            "Incomplete, untied, or unbalanced packs are not review-ready."
        )
    )
    commands = parser.add_subparsers(dest="command", required=True)
    gate = commands.add_parser("gate", help="run completeness, tie-out, and self-review controls")
    gate.add_argument(
        "--profile",
        required=True,
        choices=PROFILE_NAMES,
        help="engagement type that selects the required artefact set",
    )
    gate.add_argument("--pack", required=True, type=Path, help="directory holding the workpaper artefacts")
    gate.add_argument("--output", required=True, type=Path, help="directory for the generated readiness pack")
    gate.add_argument("--review-note", type=Path, help="optional human acknowledgement JSON")
    gate.add_argument(
        "--tieout-tolerance",
        type=_non_negative_decimal,
        default=Decimal("0.01"),
        help="maximum permitted GST or bank-rec difference",
    )
    view = commands.add_parser(
        "view", help="display an existing readiness pack after verifying its three files agree"
    )
    view.add_argument(
        "--pack-dir",
        required=True,
        type=Path,
        help="directory holding readiness-pack.json, readiness-summary.md and findings.csv",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        if exc.code == 2:
            return 1
        raise
    if args.command == "view":
        try:
            sheet, _ = render_review_sheet(args.pack_dir)
        except GateInputError as exc:
            print(f"review-ready view: verification failed: {exc}", file=sys.stderr)
            return 1
        print(sheet)
        print(
            "review-ready view: display is a review aid; it does not approve a file "
            "or change any computed status."
        )
        return 0
    destinations = {(args.output / name).resolve() for name in PACK_FILE_NAMES}
    for flag, source in (("--pack", args.pack), ("--review-note", args.review_note)):
        if source is None:
            continue
        resolved = source.resolve()
        if resolved in destinations:
            print(
                f"review-ready: output error: {flag} {source} is inside --output and "
                "shares a generated pack file name; the run would destroy it.",
                file=sys.stderr,
            )
            return 1
        if source.is_dir():
            for child in source.iterdir():
                if child.resolve() in destinations:
                    print(
                        f"review-ready: output error: {flag} contains {child.name}, which "
                        "is a generated pack file name inside --output.",
                        file=sys.stderr,
                    )
                    return 1
    try:
        pack = review_pack(
            profile=args.profile,
            pack_dir=args.pack,
            acknowledgement_path=args.review_note,
            tieout_tolerance=args.tieout_tolerance,
        )
    except (GateInputError, ValueError) as exc:
        print(f"review-ready: input error: {exc}", file=sys.stderr)
        return 1
    try:
        outputs = write_review_pack(pack, args.output)
    except (OSError, ValueError) as exc:
        print(f"review-ready: output error: {exc}", file=sys.stderr)
        return 1
    print(f"review-ready: {pack.status}; {len(pack.findings)} finding(s)")
    if pack.status == "READY" and not pack.findings:
        print("review-ready: pack may enter manager review. A human still decides.")
    for name, path in outputs.items():
        print(f"  {name}: {path}")
    return 0 if pack.status == "READY" else 2


if __name__ == "__main__":
    sys.exit(main())
