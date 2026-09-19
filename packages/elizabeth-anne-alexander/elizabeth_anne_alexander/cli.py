from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import TextIO

from .errors import GatewayError
from .gateway import evaluate, review_input_paths, validate_review
from .persist import write_evaluation, write_validation
from .util import build_root, path_within


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run a fixed-policy, synthetic Xero review boundary.")
    commands = parser.add_subparsers(dest="command", required=True)
    evaluate_parser = commands.add_parser("evaluate", help="evaluate the one allowlisted variance review")
    evaluate_parser.add_argument("--context", required=True, type=Path)
    evaluate_parser.add_argument("--request", required=True, type=Path)
    evaluate_parser.add_argument("--policy", required=True, type=Path)
    evaluate_parser.add_argument("--out", required=True, type=Path)
    decision_parser = commands.add_parser("validate-review", help="validate a human acknowledgement/escalation record")
    decision_parser.add_argument("--evidence", required=True, type=Path)
    decision_parser.add_argument("--receipt", required=True, type=Path)
    decision_parser.add_argument("--decision", required=True, type=Path, help="decision JSON under the bundled samples/ data or the working directory's build/")
    decision_parser.add_argument("--out", type=Path, help="optional JSON validation output below build/")
    return parser


def _emit(text: str, stream: TextIO) -> None:
    """Write one line without letting a path the stream cannot encode kill a finished run.

    Redirected stdout on Windows defaults to the ANSI code page, so a working
    directory holding one character outside it made the gateway exit 1 after
    the artefacts had already been written correctly. Escape the line for this
    one write instead of reconfiguring sys.stdout/sys.stderr, which would
    change the error handler for the rest of the process.
    """
    try:
        print(text, file=stream)
    except UnicodeEncodeError:
        encoding = getattr(stream, "encoding", None) or "ascii"
        print(text.encode(encoding, "backslashreplace").decode(encoding, "replace"), file=stream)


def _refuse_input_collision(out: Path, inputs: tuple[Path, ...]) -> None:
    """Refuse a validation output that names one of the files validate_review just read.

    Both sides are resolved, so a symbolic link to an input compares equal by
    path; samefile covers a hard link, whose path differs while the file does
    not. Path containment answers where a write may land, not whether the
    chosen file is expendable, and a validate-review run once overwrote its own
    decision record with the summary that said the record was valid.

    The check and the later write are two steps against a local filesystem, so
    a process that swaps a link in between them is outside what this guard
    promises. The component is a synthetic-only demonstration on one operator's
    machine; a race-proof write would need directory handles that Windows does
    not offer and is not a corner this guard claims to cover.
    """
    for protected in inputs:
        try:
            same = out == protected or (out.exists() and protected.exists() and out.samefile(protected))
        except OSError as exc:
            raise GatewayError(f"validation output cannot be checked against {protected}: {exc}.") from exc
        if same:
            raise GatewayError(f"validation output must not overwrite a review input: {protected}.")


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "evaluate":
            model, evidence, receipt = evaluate(context_path=args.context, request_path=args.request, policy_path=args.policy)
            outputs = write_evaluation(model, evidence, receipt, args.out)
            _emit(f"elizabeth-anne-alexander: REVIEW_READY; {len(model['findings'])} bounded finding(s)", sys.stdout)
            for name, path in outputs.items():
                _emit(f"  {name}: {path}", sys.stdout)
            return 0
        out = None
        if args.out:
            out = path_within(args.out, build_root(), label="validation output", require_exists=False)
            _refuse_input_collision(
                out,
                review_input_paths(evidence_path=args.evidence, receipt_path=args.receipt, decision_path=args.decision),
            )
        validation = validate_review(evidence_path=args.evidence, receipt_path=args.receipt, decision_path=args.decision)
        if out is not None:
            write_validation(
                validation,
                out,
                review_input_paths(evidence_path=args.evidence, receipt_path=args.receipt, decision_path=args.decision),
            )
        _emit(f"elizabeth-anne-alexander: {validation['status']}; {validation['decision_count']} decision(s); {validation['undecided_count']} undecided finding(s)", sys.stdout)
        return 0
    except GatewayError as exc:
        _emit(f"elizabeth-anne-alexander: blocked: {exc}", sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
