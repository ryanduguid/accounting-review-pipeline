"""Command line entry points.

Exit codes follow the repository convention: 0 clean, 2 the tool did its job
and the answer is stop, 1 the input was malformed. A halt is a 2, not a 1,
because halting is correct behaviour rather than an error.

There is deliberately no way to redact non-strictly from here: no flag, no
environment variable, no code path. ``verify._carried_placeholders`` documents
one residual limit it cannot see, a lone placeholder-shaped literal in a
document holding nothing else, and the only mitigation is that ``redact``
halts on it at the input. That mitigation is conditional on the halt being
reachable, so it holds only while every caller on the operator path redacts
strictly. Adding an escape hatch here would silently remove the guarantee.

``require_gitignored`` runs before the map is read, on every command. The map
is the key, and a key git would let you commit is the one failure this package
exists to prevent, so the check does not wait for a command that writes.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from . import entities as entities_module
from . import verify as verify_module
from .errors import EvattError, Halt
from .redact import redact
from .restore import restore
from .version import __version__


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="evatt",
        description="Pseudonymise Australian client data in markdown, locally, before sending it.",
    )
    parser.add_argument("--version", action="version", version=__version__)
    commands = parser.add_subparsers(dest="command", required=True)

    redact_parser = commands.add_parser("redact", help="raw markdown to sanitised markdown")
    redact_parser.add_argument("--in", dest="source", required=True, type=Path)
    redact_parser.add_argument("--map", dest="entity_map", required=True, type=Path)
    redact_parser.add_argument("--out", required=True, type=Path)

    restore_parser = commands.add_parser("restore", help="reverse the entity map over an answer")
    restore_parser.add_argument("--in", dest="source", required=True, type=Path)
    restore_parser.add_argument("--map", dest="entity_map", required=True, type=Path)
    restore_parser.add_argument("--out", required=True, type=Path)

    verify_parser = commands.add_parser("verify", help="re-scan a sanitised file")
    verify_parser.add_argument("--in", dest="source", required=True, type=Path)
    verify_parser.add_argument("--map", dest="entity_map", required=True, type=Path)
    return parser


def _read(path: Path) -> str:
    """Read *path* without translating its line endings.

    ``newline=""`` turns off universal newlines, so a CRLF file arrives holding
    its own ``\\r\\n`` rather than a normalised copy of itself. Paired with
    ``_write`` it is what makes a redact-then-restore round trip return the
    bytes it was given. Nothing downstream is affected: ``splitlines`` still
    reads ``\\r\\n`` as one break, so reported line numbers are unchanged, and
    the residual sweep strips the line it quotes.

    ``Path.read_text`` grew a ``newline`` parameter only in 3.13 and this
    package supports 3.10, which is why the handle is opened by hand.
    """
    with path.open(encoding="utf-8", newline="") as handle:
        return handle.read()


def _write(path: Path, text: str) -> None:
    """Write *text* to *path* exactly as it stands, creating the directory.

    ``newline=""`` again: the default rewrites every ``\\n`` to the platform's
    line ending, so on Windows redacting an LF workpaper changed every line in
    the file as well as the identifiers in it. A boundary that alters what it
    was not asked to alter is one an operator has to diff before trusting, and
    it also made the output depend on which machine ran the command.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="")


def _write_triage(path: Path, halt: Halt) -> None:
    """Write the operator's worklist for a halt.

    The summary line is the Halt's own message rather than a second sentence
    saying the same thing. One user-facing wording that appears in two places
    is one wording that drifts, and the console line below reads the same
    string, so "nothing was written" is written once, in ``errors.py``.

    ``residual`` reports every address, then every date, then every name, which
    is the order its passes run in and no use to anyone. The triage file is read
    by a person working down a document, so the candidates are sorted by the
    line they stand on. The kind and value break a tie inside one line, so the
    file is byte-identical between runs.
    """
    lines = [
        "# Triage",
        "",
        f"{halt}.",
        "",
        "Classify every candidate below into the entity map, or confirm it is",
        "noise, then run redact again.",
        "",
    ]
    for unknown in sorted(halt.unknowns, key=lambda u: (u.line, u.kind, u.value)):
        lines.append(f"- **{unknown.value}** ({unknown.kind}, line {unknown.line})")
        lines.append(f"  > {unknown.context}")
    _write(path, "\n".join(lines) + "\n")


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        entities_module.require_gitignored(args.entity_map)
        entity_map = entities_module.load(args.entity_map)
        text = _read(args.source)
    except (EvattError, OSError, UnicodeDecodeError) as error:
        print(f"error: {error}")
        return 1

    if args.command == "redact":
        # Halt is caught in its own block, ahead of anything broader. It
        # subclasses EvattError, so a wider except above it would report the
        # halt as an ordinary error and write no triage file.
        try:
            sanitised, counts = redact(text, entity_map)
        except Halt as halt:
            triage = args.out.with_name(args.out.name + ".triage.md")
            _write_triage(triage, halt)
            print(f"halted: {halt}. See {triage}")
            return 2
        _write(args.out, sanitised)
        manifest = args.out.with_name(args.out.name + ".manifest.json")
        _write(manifest, json.dumps({"schema_version": 1, "counts": counts}, indent=2) + "\n")
        print(f"wrote {args.out}")
        return 0

    if args.command == "restore":
        _write(args.out, restore(text, entity_map))
        print(f"wrote {args.out}")
        return 0

    found = verify_module.findings(text, entity_map)
    if found:
        for finding in found:
            print(f"{finding.kind}: {finding.value}")
        print(f"{len(found)} finding(s); this file is not ready to send")
        return 2
    print("no findings")
    return 0
