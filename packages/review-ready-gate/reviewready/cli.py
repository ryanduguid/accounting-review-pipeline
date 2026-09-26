from __future__ import annotations

import argparse
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .documents import create_intake, load_document, render_document
from .engine import review_pack
from .errors import GateInputError
from .profiles import PROFILE_NAMES
from .report import (
    PACK_FILE_NAMES,
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
    gate.add_argument("--document", type=Path, action="append", default=[],
                      help="synthetic document bundle to check; repeat for several documents")
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
    view.add_argument("--document", type=Path, action="append", default=[],
                      help="display document evidence bound to this pack, in the original order")
    intake = commands.add_parser("intake", help="prepare a fabricated labelled-invoice text bundle")
    intake.add_argument("--synthetic", action="store_true", required=True,
                        help="confirm that the input is fabricated data")
    intake.add_argument("--source", type=Path, required=True, help="original .txt or .pdf document")
    intake.add_argument("--text", type=Path, required=True, help="UTF-8 text; form feed separates pages")
    intake.add_argument("--text-origin", choices=("manual", "pdf-inspector"), required=True)
    intake.add_argument("--text-version", default="", help="required version for pdf-inspector text")
    intake.add_argument("--entity", required=True, help="entity matching the trial balance Tenant")
    intake.add_argument("--period-end", required=True, help="review period in YYYY-MM-DD format")
    intake.add_argument("--output", type=Path, required=True, help="new directory outside a checkout")
    document = commands.add_parser("view-document", help="verify and display a document bundle")
    document.add_argument("--bundle", type=Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        if exc.code == 2:
            return 1
        raise
    if args.command == "intake":
        try:
            output = create_intake(source_path=args.source, text_path=args.text,
                                   output_dir=args.output, entity=args.entity,
                                   period_end=args.period_end, text_origin=args.text_origin,
                                   text_version=args.text_version)
        except (GateInputError, OSError, ValueError) as exc:
            print(f"review-ready intake: {exc}", file=sys.stderr)
            return 1
        print(f"review-ready intake: proposed fields written to {output}; human review required.")
        return 0
    if args.command == "view-document":
        try:
            print(render_document(load_document(args.bundle)))
        except GateInputError as exc:
            print(f"review-ready view-document: verification failed: {exc}", file=sys.stderr)
            return 1
        return 0
    if args.command == "view":
        try:
            sheet, _ = render_review_sheet(args.pack_dir, document_paths=tuple(args.document))
        except GateInputError as exc:
            print(f"review-ready view: verification failed: {exc}", file=sys.stderr)
            return 1
        print(sheet)
        print(
            "review-ready view: display is a review aid; it does not approve a file "
            "or change any computed status."
        )
        return 0
    # Both output guards run before any client file is opened. write_review_pack
    # repeats this one so a library caller cannot get past it, but a reviewer who
    # mistyped --output should hear about it before the run reads a workpaper
    # pack, not after.
    try:
        require_output_outside_repository(args.output)
    except GateInputError as exc:
        print(f"review-ready: output error: {exc}", file=sys.stderr)
        return 1
    destinations = {(args.output / name).resolve() for name in PACK_FILE_NAMES}
    for flag, source in (
        ("--pack", args.pack), ("--review-note", args.review_note),
        *(("--document", path) for path in args.document),
    ):
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
            try:
                children = list(source.iterdir())
            except OSError as exc:
                print(f"review-ready: input error: cannot inspect {flag}: {exc}", file=sys.stderr)
                return 1
            for child in children:
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
            document_paths=tuple(args.document),
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
    if pack.controls_not_run:
        # Printed before the READY line, because it is what qualifies it: a
        # control with no input reached no verdict, and READY covers only the
        # controls that ran.
        print(
            f"review-ready: {len(pack.controls_not_run)} control(s) did not run: "
            + ", ".join(control.slot for control in pack.controls_not_run)
        )
    if pack.status == "READY" and not pack.findings:
        print("review-ready: pack may enter manager review. A human still decides.")
    for name, path in outputs.items():
        print(f"  {name}: {path}")
    return 0 if pack.status == "READY" else 2


if __name__ == "__main__":
    sys.exit(main())
