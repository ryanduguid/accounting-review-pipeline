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

The triage path is guarded too, the way ``entities.save`` guards the map's
.tmp: immediately before a byte is written to it. It carries whole unredacted
residual lines, and ``--out`` derives it, so the operator never typed the path
that would leak. It is guarded there rather than up front because it is an
output, not the key: a run that never halts never writes one, and failing a
clean redact over a file it was not going to create would be a refusal about
nothing. ``restore --out`` is deliberately not guarded. That path is the
operator's own choice for a document of real names, it can be anywhere, and a
guard on it would demand a covering rule for every answer file anyone ever
restores. The README says so rather than the code pretending otherwise.

Errors go to stderr, where argparse already writes its own, so a caller
redirecting stdout to the sanitised text still sees why a run failed.
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
import stat
import sys
from pathlib import Path
from typing import NoReturn

from . import disclosure
from . import entities as entities_module
from . import verify as verify_module
from .errors import EvattError, Halt
from .patterns import value_pattern
from .redact import redact
from .restore import restore
from .version import __version__


class _ArgumentParser(argparse.ArgumentParser):
    private_errors = False

    def error(self, message: str) -> NoReturn:
        if self.private_errors or self.prog.endswith((" disclosure-record", " disclosure-check")):
            message = "invalid disclosure arguments"
        super().error(message)


def build_parser(*, private_errors: bool = False) -> argparse.ArgumentParser:
    parser = _ArgumentParser(
        prog="evatt",
        description="Pseudonymise Australian client data in markdown, locally, before sending it.",
    )
    parser.private_errors = private_errors
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

    for name, help_text in (
        ("disclosure-record", "record local evidence for an intended disclosure"),
        ("disclosure-check", "check local evidence; does not authenticate permission"),
    ):
        command = commands.add_parser(name, help=help_text)
        command.add_argument("--in", dest="source", required=True, type=Path)
        command.add_argument("--map", dest="entity_map", required=True, type=Path)
        command.add_argument("--destination", required=True)
        command.add_argument("--decision-ref", required=True)
        command.add_argument("--out" if name == "disclosure-record" else "--record",
                             required=True, type=Path)
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
    parameter only in 3.13 and this package supports 3.11, which is why the
    handle is opened by hand.
    """
    with path.open(encoding="utf-8", newline="") as handle:  # NOSONAR: local CLI path chosen by the operator
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
    path.write_text(text, encoding="utf-8", newline="")  # NOSONAR: local CLI path chosen by the operator


def _remove(path: Path) -> None:
    """Delete *path* if it is there, and say nothing if it is not."""
    path.unlink(missing_ok=True)  # NOSONAR: local CLI path chosen by the operator


def _collision(args: argparse.Namespace) -> str | None:
    """Name a file collision this run would mutate, or None when paths are distinct.

    ``redact --map entities.json --out entities.json`` exited 0 and left the map
    holding redacted markdown. The map is the only copy of the key, structured
    identifiers are replaced one way, and every document already redacted
    against that map becomes unrestorable, so one mistyped path was
    unrecoverable data loss. ``--out`` equal to ``--in`` destroys the source the
    same way.

    The 2 paths ``--out`` derives are checked as well. Neither is spelt on the
    command line, so neither collision is one the operator can see coming: a map
    named ``out.md.manifest.json`` beside ``--out out.md`` is overwritten by the
    manifest, and an input named ``out.md.triage.md`` beside the same ``--out``
    is overwritten by the triage file.

    Outputs must also be distinct from each other. A manifest hard-linked to
    the document replaces its text with JSON, while removing a triage path
    targeted by an output symlink leaves that output dangling.
    """
    if args.command == "verify":
        return None
    protected = [
        (args.entity_map.resolve(), "the entity map"),
        (args.source.resolve(), "the input"),
    ]
    written = [("--out", args.out)]
    if args.command == "redact":
        written.append(("the manifest path --out derives", _manifest_path(args.out)))
        written.append(("the triage path --out derives", _triage_path(args.out)))
    for name, path in written:
        resolved = path.resolve()
        for candidate, description in protected:
            if resolved == candidate or (
                resolved.exists() and candidate.exists() and resolved.samefile(candidate)
            ):
                return f"{name} is {description}, {resolved}; refusing to overwrite it"
        protected.append((resolved, name))
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
    ``redact`` is what guarantees that; the caller asks git whether it would let
    you commit this path before a byte reaches it, because the file still names
    every candidate it wants classified.

    The candidate value is requoted with its whitespace collapsed. A candidate
    can hold a newline, because NAME's ``\\s+`` spans one and a hard-wrapped
    "Jane\\nRoe" is a single candidate, and writing that raw broke the markdown
    bold span open across two lines. The context is a single line already, so
    only the value needs it.
    """
    lines = [
        "# Triage",
        "",
        f"{halt}.",
        "",
        "Classify every candidate, then run redact again.",
        "Map a false positive as an entity; it will also be replaced and restored.",
        "For a number: rewrite a TFN, ABN, ACN or Medicare number in a recognised spelling",
        "so pass one replaces it one-way; map any other number as an entity.",
        "",
    ]
    quoted_lines: set[int] = set()
    for unknown in sorted(halt.unknowns, key=lambda u: (u.line, u.kind, u.value)):
        value = " ".join(unknown.value.split())
        lines.append(f"- **{value}** ({unknown.kind}, line {unknown.line})")
        if unknown.line not in quoted_lines:
            lines.append(f"  > {unknown.context}")
            quoted_lines.add(unknown.line)
    _write(path, "\n".join(lines) + "\n")


def _record_path(path: Path) -> None:
    """Reject static links in the record path; the containing worktree must be trusted."""
    get_attributes = None
    if sys.platform == "win32":
        # lstat can follow non-name-surrogate reparse points on Windows.
        # Query the entry attributes before allowing any such traversal.
        get_attributes = ctypes.WinDLL("kernel32", use_last_error=True).GetFileAttributesW
        get_attributes.argtypes = [ctypes.c_wchar_p]
        get_attributes.restype = ctypes.c_uint32

    def inspect(candidate: Path) -> None:
        if sys.platform == "win32" and get_attributes is not None:
            attributes = get_attributes(str(candidate))
            if attributes == 0xFFFFFFFF:
                error = ctypes.get_last_error()
                if error in (2, 3):  # File or parent directory does not exist.
                    return
                raise ctypes.WinError(error)
            if attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                raise EvattError("disclosure record paths must not use links")
        try:
            entry = candidate.lstat()
        except FileNotFoundError:
            return
        if stat.S_ISLNK(entry.st_mode):
            raise EvattError("disclosure record paths must not use links")

    lexical = (path, *path.parents)
    for candidate in lexical:
        inspect(candidate)
    # A missing component before '..' can conceal an existing linked prefix.
    normalised = tuple(Path(os.path.abspath(candidate)) for candidate in lexical)
    for candidate in (*normalised, *normalised[-1].parents):
        inspect(candidate)


def _read_record(path: Path) -> bytes:
    _record_path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
        raise EvattError("disclosure record must be a regular file with one link")
    flags = (os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
             | getattr(os, "O_NONBLOCK", 0))
    with os.fdopen(os.open(path, flags), "rb") as handle:  # NOSONAR: local CLI path chosen by the operator
        opened = os.fstat(handle.fileno())
        if not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1:
            raise EvattError("disclosure record must be a regular file with one link")
        return handle.read(disclosure.MAX_RECORD_BYTES + 1)


def _disclosure_command(args: argparse.Namespace) -> int:
    # Fixed diagnostics cover map validation, git, paths and malformed data.
    # Those errors can otherwise quote the very values this boundary protects.
    try:
        entities_module.require_gitignored(args.entity_map)
        entity_map = entities_module.load(args.entity_map)
    except (OSError, ValueError, RuntimeError):
        return _fail("cannot use entity map; it must be valid, ignored and untracked")
    try:
        payload = args.source.read_bytes()
        if args.command == "disclosure-record":
            record = disclosure.create_record(payload, entity_map,
                                              destination=args.destination,
                                              decision_ref=args.decision_ref)
            protected = (args.source, args.entity_map, _manifest_path(args.source),
                         _triage_path(args.source))
            if any(args.out.resolve() == path.resolve() for path in protected):
                raise EvattError("disclosure record path collides with a protected file")
            _record_path(args.out)
            args.out.parent.mkdir(parents=True, exist_ok=True)
            entities_module.require_gitignored(args.out, "the disclosure record")
            # Exclusive creation never truncates an existing record or alias.
            # A failed write may leave an incomplete record, which checking rejects.
            flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
            with os.fdopen(os.open(args.out, flags, 0o600), "wb") as handle:  # NOSONAR: local CLI path chosen by the operator
                if handle.write(record) != len(record):
                    raise OSError("incomplete disclosure record write")
            print("Local disclosure evidence recorded. External authorisation has not been checked.")
        else:
            entities_module.require_gitignored(args.record, "the disclosure record")
            disclosure.check_record(payload, entity_map, _read_record(args.record),
                                    expected_destination=args.destination,
                                    expected_decision_ref=args.decision_ref)
            print("Local disclosure evidence matches. External authorisation has not been checked.")
    except disclosure.DisclosureRefused as error:
        print(f"refused: {error}", file=sys.stderr)
        return 2
    except EvattError:
        return _fail("invalid disclosure input or record path; use local ignored, untracked records")
    except (OSError, UnicodeError, RuntimeError):
        return _fail("cannot read or create disclosure files; existing files are never overwritten")
    return 0


def main(argv: list[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    try:
        # Omitted or misspelt commands and argparse's option abbreviations
        # must not expose disclosure identifiers in argument diagnostics.
        private = any(
            argument.startswith("disclosure-")
            or (argument.startswith("--") and argument != "--" and any(
                option.startswith(argument.partition("=")[0])
                for option in ("--destination", "--decision-ref", "--record")
            ))
            for argument in arguments
        )
        args = build_parser(private_errors=private).parse_args(arguments)
    except SystemExit as request:
        # --help and --version are argparse doing what it was asked; anything
        # else is a usage error, and a usage error is malformed input, not a
        # halt. Letting argparse's own 2 through would make a typo
        # indistinguishable from a document that needs triage.
        if request.code in (0, None):
            raise
        return 1

    if args.command in ("disclosure-record", "disclosure-check"):
        return _disclosure_command(args)

    try:
        collision = _collision(args)
        if collision is not None:
            return _fail(collision)
        entities_module.require_gitignored(args.entity_map)
        entity_map = entities_module.load(args.entity_map)
        text, ending = _read(args.source)
    except (EvattError, OSError, UnicodeDecodeError) as error:
        return _fail(error)

    if args.command == "redact":
        manifest = _manifest_path(args.out)
        triage = _triage_path(args.out)
        # Halt is caught in its own block, ahead of anything broader. It
        # subclasses EvattError, so a wider except above it would report the
        # halt as an ordinary error and write no triage file.
        try:
            sanitised, counts = redact(text, entity_map)
        except Halt as halt:
            # The stale output goes first. "Nothing was written" has to be true
            # of the directory, not just of this run, and an earlier run's
            # output is what an operator would send. Clearing it before the
            # triage file is written keeps that true even when writing the
            # triage file is what fails.
            try:
                _remove(args.out)
                _remove(manifest)
            except OSError as error:
                return _fail(error)
            # The triage file holds whole unredacted residual lines at a path
            # the operator never spelt, so it is held to the same standard as
            # the map and the map's .tmp: git is asked before a byte is written,
            # and an answer of "yes, you could commit this" is a refusal rather
            # than a file.
            try:
                triage.parent.mkdir(parents=True, exist_ok=True)
                entities_module.require_gitignored(triage, "the triage file")
                _write_triage(triage, halt)
            except (EvattError, OSError) as error:
                return _fail(error)
            print(f"halted: {halt}. See {triage}")
            return 2
        try:
            _write(args.out, sanitised, ending)
        except OSError as error:
            return _fail(error)
        try:
            _write(manifest, json.dumps({"schema_version": 1, "counts": counts}, indent=2) + "\n")
            # An earlier run's triage file names real people in plaintext, and
            # it sits in the directory the operator is about to send from. This
            # run answered it, so it goes. Same argument as the halt path's
            # removal of a stale output, in the other direction: what the
            # operator sees is a directory, not one run's exit code.
            _remove(triage)
        except OSError as error:
            # A sanitised document with no manifest is a document nobody can
            # say what was replaced in, so it does not survive the failure.
            # Nor does one left beside a triage file this run could not clear.
            try:
                _remove(args.out)
                _remove(manifest)
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
            match = value_pattern(finding.value).search(text)
            location = f"line {text.count(chr(10), 0, match.start()) + 1}" if match else "detected in input"
            print(f"{finding.kind}: {location}")
        print(f"{len(found)} finding(s); this file is not ready to send")
        return 2
    print("no findings")
    return 0
