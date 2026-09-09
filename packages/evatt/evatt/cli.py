"""Command line entry points.

Exit codes follow the repository convention: 0 clean, 2 the tool did its job
and the answer is stop, 1 the input was malformed. A halt is a 2, not a 1,
because halting is correct behaviour rather than an error. ``parse_args`` is
caught for the same reason: argparse exits 2 on a missing option, an unknown
subcommand or no subcommand at all, and a wrapper reading 2 as "halted or
findings" would take a typo for a document needing triage.

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

Errors go to stderr, where argparse already writes its own, so a caller
redirecting stdout to the sanitised text still sees why a run failed.
"""
from __future__ import annotations

import argparse
import json
import sys
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


def _manifest_path(out: Path) -> Path:
    return out.with_name(out.name + ".manifest.json")


def _triage_path(out: Path) -> Path:
    return out.with_name(out.name + ".triage.md")


def _fail(message: object) -> int:
    print(f"error: {message}", file=sys.stderr)
    return 1


def _read(path: Path) -> tuple[str, str]:
    """Read *path* as LF text, with the line ending to give it back on the way out.

    Detection no longer depends on this. ``redact`` and ``verify.findings``
    each normalise their own input, because they are public and every other
    caller needs the same guard; those two docstrings carry the reasoning. What
    is normalised here is still load-bearing for ``restore``, which is a
    placeholder substitution that neither reads nor rewrites line endings: it
    hands back whatever it was given, and ``_write`` then expands every ``\\n``
    to the source's ending, so a CRLF pair reaching it would come out ``\\r\\r\\n``.
    Normalising on the way in is one line and keeps all three commands feeding
    ``_write`` the LF text it expects.

    The dominant ending is returned so ``_write`` can restore it, which keeps
    the round trip byte-exact for the ordinary file that uses one ending
    throughout. A file that mixes endings is normalised to its dominant one
    rather than preserved: this is a boundary that rewrites identifiers, not a
    byte-level editor, and reproducing a mixture nothing meant to create is not
    worth carrying an offset map for. A lone ``\\r`` is left where it stands.

    ``newline=""`` turns off universal newlines so the endings can be counted
    before anything is decided about them. ``Path.read_text`` grew a ``newline``
    parameter only in 3.13 and this package supports 3.10, which is why the
    handle is opened by hand.
    """
    with path.open(encoding="utf-8", newline="") as handle:
        raw = handle.read()
    crlf = raw.count("\r\n")
    ending = "\r\n" if crlf > raw.count("\n") - crlf else "\n"
    return raw.replace("\r\n", "\n"), ending


def _write(path: Path, text: str, ending: str = "\n") -> None:
    """Write *text* to *path* with *ending* for every break, creating the directory.

    ``newline=""`` disables the platform rewrite, so what lands on disk is what
    was asked for and not what Windows would have preferred. The default is LF
    because the manifest and the triage file are this tool's own output and
    should not depend on which machine ran the command; only the redacted or
    restored document is written back with the ending its source carried.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    if ending != "\n":
        text = text.replace("\n", ending)
    path.write_text(text, encoding="utf-8", newline="")


def _remove(path: Path) -> None:
    """Delete *path* if it is there, and say nothing if it is not."""
    path.unlink(missing_ok=True)


def _collision(args: argparse.Namespace) -> str | None:
    """Name the protected file this run would overwrite, or None when it is safe.

    ``redact --map entities.json --out entities.json`` exited 0 and left the map
    holding redacted markdown. The map is the only copy of the key, structured
    identifiers are replaced one way, and every document already redacted
    against that map becomes unrestorable, so one mistyped path was
    unrecoverable data loss. ``--out`` equal to ``--in`` destroys the source the
    same way.

    The two paths ``--out`` derives are checked as well. They are not spelt on
    the command line, so ``--out entities`` beside a map at ``entities.json``
    looks harmless and is not: the manifest would land on the map.
    """
    if args.command == "verify":
        return None
    protected = (
        (args.entity_map.resolve(), "the entity map"),
        (args.source.resolve(), "the input"),
    )
    written = [("--out", args.out)]
    if args.command == "redact":
        written.append(("the manifest path --out derives", _manifest_path(args.out)))
        written.append(("the triage path --out derives", _triage_path(args.out)))
    for name, path in written:
        resolved = path.resolve()
        for candidate, description in protected:
            if resolved == candidate:
                return f"{name} is {description}, {resolved}; refusing to overwrite it"
    return None


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

    Every context quoted here comes from the redacted text, never the input.
    This file sits in the directory the operator is about to send from, so a
    context carrying a pre-redaction line would put a real tax file number, an
    email and a mapped client name in plaintext beside the sanitised document.
    ``redact`` is what guarantees that; ``*.triage.md`` is gitignored as well,
    because the file still names the candidates it wants classified.
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
    try:
        args = build_parser().parse_args(argv)
    except SystemExit as request:
        # --help and --version are argparse doing what it was asked; anything
        # else is a usage error, and a usage error is malformed input, not a
        # halt. Letting argparse's own 2 through would make a typo
        # indistinguishable from a document that needs triage.
        if request.code in (0, None):
            raise
        return 1

    collision = _collision(args)
    if collision is not None:
        return _fail(collision)

    try:
        entities_module.require_gitignored(args.entity_map)
        entity_map = entities_module.load(args.entity_map)
        text, ending = _read(args.source)
    except (EvattError, OSError, UnicodeDecodeError) as error:
        return _fail(error)

    if args.command == "redact":
        manifest = _manifest_path(args.out)
        # Halt is caught in its own block, ahead of anything broader. It
        # subclasses EvattError, so a wider except above it would report the
        # halt as an ordinary error and write no triage file.
        try:
            sanitised, counts = redact(text, entity_map)
        except Halt as halt:
            triage = _triage_path(args.out)
            try:
                _write_triage(triage, halt)
                # "nothing was written" has to be true of the directory, not
                # just of this run. An earlier run's output sitting beside the
                # new triage file is what an operator would send, so it goes.
                _remove(args.out)
                _remove(manifest)
            except OSError as error:
                return _fail(error)
            print(f"halted: {halt}. See {triage}")
            return 2
        try:
            _write(args.out, sanitised, ending)
        except OSError as error:
            return _fail(error)
        try:
            _write(manifest, json.dumps({"schema_version": 1, "counts": counts}, indent=2) + "\n")
        except OSError as error:
            # A sanitised document with no manifest is a document nobody can
            # say what was replaced in, so it does not survive the failure.
            try:
                _remove(args.out)
            except OSError:
                pass
            return _fail(error)
        print(f"wrote {args.out}")
        return 0

    if args.command == "restore":
        try:
            _write(args.out, restore(text, entity_map), ending)
        except OSError as error:
            return _fail(error)
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
