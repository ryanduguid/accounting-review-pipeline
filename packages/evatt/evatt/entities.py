"""The local entity map: real value to stable placeholder.

The map is the key. It stays on disk, gitignored, and is never transmitted.
Placeholders are typed and stable so that the receiving model can still reason
across a document, and so ``restore`` can put real names back into its answer.

Ordinals are stored rather than derived at read time. Deriving them from
iteration order would make output depend on dictionary ordering, and the
determinism property in the test suite exists to catch exactly that. They are
monotonic while the map is append-only. Keep retired entries: deleting the
highest ordinal lets the next assignment reuse it.
"""
from __future__ import annotations

import json
import os
import re
import secrets
import subprocess
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from types import MappingProxyType
from typing import BinaryIO, Mapping, Sequence

from .errors import EvattError
from .patterns import PLACEHOLDER_CI

SCHEMA_VERSION = 1
KINDS = ("client", "person", "staff", "entity")
_PREFIX = {"client": "CLIENT", "person": "PERSON", "staff": "STAFF", "entity": "ENTITY"}
_REQUIRED = {"value", "placeholder", "kind", "added"}

# Exclusive creation is the whole of the temporary file's protection, so a
# platform that cannot offer it is refused rather than written to. O_CREAT and
# O_EXCL are present on every platform CPython supports; the check is here so
# that the day one is not, ``save`` stops instead of silently opening a
# pathname something else may already own.
_EXCLUSIVE_CREATION = hasattr(os, "O_CREAT") and hasattr(os, "O_EXCL")
# O_NOFOLLOW is POSIX only and O_BINARY and O_NOINHERIT are Windows only, so
# each is taken if the platform has it. Only O_CREAT|O_EXCL is load-bearing:
# exclusive creation fails on a pathname that already exists, and a symbolic
# link, a dangling symbolic link and a hard link are all pathnames that already
# exist. O_NOFOLLOW adds nothing to that on POSIX and has no Windows spelling,
# which is why its absence there costs no guarantee. O_BINARY keeps Windows from
# translating the LF the map is written with, and O_NOINHERIT keeps a plaintext
# copy of the key out of a child process this package did not start.
_TEMPORARY_FLAGS = (
    os.O_WRONLY
    | getattr(os, "O_CREAT", 0)
    | getattr(os, "O_EXCL", 0)
    | getattr(os, "O_NOFOLLOW", 0)
    | getattr(os, "O_BINARY", 0)
    | getattr(os, "O_NOINHERIT", 0)
)
# Attempts at a unique name before giving up. A collision means the name was
# taken between minting and opening it, which eight 32-bit tokens in a row do
# not lose to by accident; a run that does is being raced, and stopping is the
# answer either way.
_TEMPORARY_ATTEMPTS = 8


@dataclass(frozen=True)
class Entity:
    value: str
    placeholder: str
    kind: str
    added: str


def _placeholder_for(kind: str, ordinal: int) -> str:
    return f"{_PREFIX[kind]}_{ordinal:02d}"


def _fold(value: str) -> str:
    """The comparison form of a value: whitespace runs collapsed, then case-folded.

    ``patterns.value_pattern`` matches a value case-insensitively and renders
    every whitespace run as ``\\s+``, so "Jane Roe", "jane roe", "JANE ROE",
    "Jane  Roe" and a hard-wrapped "Jane\\nRoe" are one name to pass two. Two
    map entries that fold together would therefore claim two placeholders for
    one person while a single pattern matched every occurrence, and whichever
    entry the leftmost-longest resolution reached first would take the lot. The
    other one would then sit in the map looking assigned and never appear in a
    document, which is how a restore puts the wrong name back.

    Casefold rather than lower, because it is the folding that compares "STRASSE"
    with "strasse", and the value may be any Latin-1 name the token class
    accepts.
    """
    return " ".join(value.split()).casefold()


def _check_fields(
    value: object, kind: object, added: object, folded: Mapping[str, str] = MappingProxyType({})
) -> tuple[str, str]:
    """Validate the fields ``load``, ``save`` and ``assign`` share, so they cannot drift.

    An ``assign`` that mints an entity ``load`` would later reject turns the map
    into a file that cannot be read back, and the map is the only copy of the key.

    A value shaped like an assigned placeholder is refused outright. Nothing
    downstream guards the shape of a value: pass two compiles the raw value into
    a pattern, so a map holding the value "TFN_01" rewrites the placeholder pass
    one has just written, destroys the only record that a tax file number was
    there, and leaves a manifest that still counts the tfn. Refusing it here
    keeps it out of the map rather than repairing the damage later. The shape
    test is ``PLACEHOLDER_CI`` rather than ``PLACEHOLDER``, because the pattern
    pass two compiles is case-insensitive: while this guard was case-sensitive
    it accepted "tfn_01" and "Tfn_01", and those did the same damage.

    *folded* maps each already-accepted value's fold to the value it came from.
    A new value that folds onto one of them is refused, which is what keeps two
    spellings of one name from claiming two placeholders. ``load`` supplies it,
    entry by entry, so the rejection covers a hand-edited map as well as one
    this package wrote. ``assign`` supplies nothing, because it resolves the
    fold to the existing entity and returns it before it gets here.

    The exact-duplicate case is a fold collision too, and is reported by this
    check rather than by a separate one: one comparison, so the two cannot
    disagree about what counts as the same value.

    The accepted value and kind come back narrowed to ``str``, so a caller that
    goes on to fold the value or look the kind's prefix up does it on what this
    function checked rather than on an object it has to re-examine.
    """
    if not isinstance(value, str) or not value.strip():
        raise EvattError("entity value must be a non-empty string")
    if PLACEHOLDER_CI.search(value):
        raise EvattError(f"entity value {value!r} is shaped like an assigned placeholder")
    if not isinstance(kind, str) or kind not in KINDS:
        raise EvattError(f"unknown entity kind {kind!r}")
    try:
        # isoformat() round trips only an exact YYYY-MM-DD calendar date, which
        # rejects both 2026-13-45 and the compact and week forms fromisoformat
        # accepts from Python 3.11.
        exact = isinstance(added, str) and date.fromisoformat(added).isoformat() == added
    except ValueError:
        exact = False
    if not exact:
        raise EvattError(f"entity added date must be YYYY-MM-DD, got {added!r}")
    # Last, so the field errors keep the precedence they had: a malformed entry
    # is reported as malformed rather than as a collision with a good one.
    previous = folded.get(_fold(value))
    if previous is not None:
        if previous == value:
            raise EvattError(f"duplicate entity value {value!r}")
        raise EvattError(
            f"entity value {value!r} is {previous!r} again once case and whitespace "
            "are folded; pass two matches them as one value, so they cannot hold "
            "two placeholders"
        )
    return value, kind


def _check_entry(
    value: object,
    placeholder: object,
    kind: object,
    added: object,
    seen_values: dict[str, str],
    seen_placeholders: set[str],
) -> None:
    """Validate one entry against the entries already accepted, then record it.

    ``load`` and ``save`` both check an entry with this, one entry at a time, so
    a map ``save`` writes is a map ``load`` can read back. ``save`` used to
    serialise whatever ``Sequence[Entity]`` it was handed: a library caller
    could atomically replace the only local copy of the key with a document
    every later command refuses to load, and nothing said so until the next run.

    The two collections are threaded in rather than rebuilt here, because both
    duplicate checks are about the entries already seen and the caller owns the
    order they are seen in. They are updated here as well, so a caller cannot
    check an entry and then forget to record it.
    """
    value_text, kind_text = _check_fields(value, kind, added, seen_values)
    # The full shape, not just the prefix: a map holding both CLIENT_1 and
    # CLIENT_10 would let naive replacement corrupt the longer placeholder.
    if not isinstance(placeholder, str) or not re.fullmatch(
        rf"{_PREFIX[kind_text]}_[0-9]{{2,}}", placeholder
    ):
        raise EvattError(f"placeholder {placeholder!r} does not match kind {kind!r}")
    if placeholder in seen_placeholders:
        raise EvattError(f"duplicate placeholder {placeholder!r}")
    seen_values[_fold(value_text)] = value_text
    seen_placeholders.add(placeholder)


def _check_sequence(entities: Sequence[Entity]) -> None:
    """Validate a whole map the way ``load`` validates a whole file.

    The state starts empty and is carried across the sequence, so the duplicate
    value and duplicate placeholder checks see the same thing they see in
    ``load``: everything accepted before this entry.
    """
    seen_values: dict[str, str] = {}
    seen_placeholders: set[str] = set()
    for entity in entities:
        _check_entry(
            entity.value,
            entity.placeholder,
            entity.kind,
            entity.added,
            seen_values,
            seen_placeholders,
        )


def load(path: Path) -> tuple[Entity, ...]:
    """Load and strictly validate the map. Every rejection is a hard error."""
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvattError(f"cannot read entity map {path}: {error}") from error
    if not isinstance(document, dict) or set(document) != {"schema_version", "entries"}:
        raise EvattError("entity map must hold exactly schema_version and entries")
    version = document["schema_version"]
    # An exact int, because True == 1 and 1.0 == 1 would otherwise both pass.
    if type(version) is not int or version != SCHEMA_VERSION:
        raise EvattError(f"unsupported entity map schema {version!r}")
    if not isinstance(document["entries"], list):
        raise EvattError("entity map entries must be a list")

    loaded: list[Entity] = []
    # Folded value to the value it came from, so a rejection can name both.
    seen_values: dict[str, str] = {}
    seen_placeholders: set[str] = set()
    for entry in document["entries"]:
        if not isinstance(entry, dict) or set(entry) != _REQUIRED:
            raise EvattError(f"entity map entry must hold exactly {sorted(_REQUIRED)}")
        value, placeholder = entry["value"], entry["placeholder"]
        kind, added = entry["kind"], entry["added"]
        _check_entry(value, placeholder, kind, added, seen_values, seen_placeholders)
        loaded.append(Entity(value=value, placeholder=placeholder, kind=kind, added=added))
    return tuple(loaded)


def _open_temporary(path: Path) -> tuple[Path, BinaryIO]:
    """Create a temporary beside *path* that cannot be a pathname something else owns.

    ``save`` used to open the fixed name ``<map>.tmp`` in write mode. Anything
    already sitting at that name was opened and truncated, so a symbolic or hard
    link left there, at a name inside a directory the operator's own tools write
    to, made ``save`` overwrite the link's target and then move the map on top of
    it. Both files are ignored by construction, which is exactly why nobody
    would notice: the destroyed one is never in a diff.

    The name is unique per attempt and the file is created exclusively, so no
    pre-existing pathname is ever opened for writing. Exclusive creation is what
    carries that, not the uniqueness: a name that exists, as any link does,
    fails the create rather than being followed. The uniqueness is what keeps a
    temporary left behind by a hard kill from wedging every later save, and what
    lets two saves of two different maps in one directory proceed.

    The name keeps ``<map>.tmp`` as its prefix and ``.tmp`` as its suffix. The
    suffix means one ``*.tmp`` rule still covers every temporary this writes, so
    no ignore rule written for the old fixed name has to change, and the prefix
    means a refusal still names the file the operator's rule was written for.

    The handle owns the descriptor, so closing it closes the descriptor once.
    """
    if not _EXCLUSIVE_CREATION:
        raise EvattError(
            "this platform cannot create a file exclusively, so the entity map "
            "temporary cannot be written without the risk of truncating whatever "
            "already holds that name; refusing to save"
        )
    for _attempt in range(_TEMPORARY_ATTEMPTS):
        temporary = path.with_name(f"{path.name}.tmp.{secrets.token_hex(4)}.tmp")
        # The temporary holds the same real values as the map, so it is held to
        # the same standard before a byte is written to it. The caller removes
        # it after any Python-level failure, but a hard kill, a container stop
        # or a power loss all leave it on disk, so "it is short lived" is not a
        # reason to let a plaintext copy of the key sit at a committable path.
        # Guarding whatever name is actually written, rather than naming one
        # .tmp in one .gitignore, is what makes this correct wherever the map
        # lives, and it means ``save`` requires a work tree.
        require_gitignored(temporary, "the entity map temporary")
        try:
            # 0o600 keeps the plaintext copy of the key to its owner on POSIX.
            # Windows honours only the write bit of a mode, so there the
            # temporary inherits the directory's permissions like any other
            # file, and the directory is what has to be private. The README
            # says so rather than the code implying a protection it has not got.
            descriptor = os.open(temporary, _TEMPORARY_FLAGS, 0o600)
        except FileExistsError:
            continue
        try:
            return temporary, os.fdopen(descriptor, "wb")
        except OSError:
            os.close(descriptor)
            temporary.unlink(missing_ok=True)
            raise
    raise EvattError(
        f"cannot create a temporary beside {path} after {_TEMPORARY_ATTEMPTS} attempts; "
        "something is taking the names as fast as they are minted, so the map is not written"
    )


def save(path: Path, entities: Sequence[Entity]) -> None:
    """Write the map atomically, with a trailing newline and stable key order.

    A plain write truncates before it fills. If that is interrupted, the only
    copy of the key is gone, so write a neighbouring temporary file and rename.

    Every entry is validated first, against exactly the checks ``load`` applies,
    and nothing is created or written until they all pass. Serialising a
    sequence ``load`` would reject replaced the only local copy of the key with
    a document every later command fails on, which is the same loss as the
    truncation the temporary exists to prevent, arriving by a different route.

    The temporary is flushed and fsynced through its own descriptor before the
    rename, because a rename that reaches the disk ahead of the data it points
    at produces exactly the truncated map the temporary exists to prevent.

    Bytes are written rather than text, so the map is LF on every platform
    instead of CRLF on Windows and LF elsewhere. The file is this package's own
    and nothing reads it by line, so one encoding of one document is worth more
    than matching a local convention.
    """
    _check_sequence(entities)
    # Write to the same resolved target that the ignore guard checks. A map
    # reached through a symbolic link is written at the link's target, and the
    # link is left standing.
    path = path.resolve()
    require_gitignored(path)
    document = {
        "schema_version": SCHEMA_VERSION,
        "entries": [
            {"value": e.value, "placeholder": e.placeholder, "kind": e.kind, "added": e.added}
            for e in entities
        ],
    }
    text = json.dumps(document, indent=2, ensure_ascii=False) + "\n"
    temporary, handle = _open_temporary(path)
    try:
        with handle:
            handle.write(text.encode("utf-8"))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def assign(entities: Sequence[Entity], value: str, kind: str, added: str) -> Entity:
    """Return the mapping for *value*, minting the next ordinal only if it is new.

    Returning the existing entity keeps one real value on one placeholder; a
    second placeholder for the same value would fail ``load``'s duplicate check.
    Ordinals are max+1 within the kind. Keep retired entries in the map so a
    later assignment cannot reuse a deleted entry's placeholder.

    The match is on ``_fold``, not on an exact string. An operator working a
    triage file types what the document showed them, and the document may have
    shouted the name, wrapped it across a line or lower-cased it. Pass two
    matches all of those as the one mapped value, so minting a second
    placeholder for the second spelling would put two placeholders on one
    person and leave one of them meaning nothing. The entity that comes back
    keeps the spelling the map already holds.

    Pure: it does not mutate *entities*. The caller appends and saves.
    """
    _check_fields(value, kind, added)
    for existing in entities:
        if _fold(existing.value) == _fold(value):
            return existing
    prefix = _PREFIX[kind] + "_"
    ordinals = [
        int(e.placeholder[len(prefix):])
        for e in entities
        if e.placeholder.startswith(prefix) and e.placeholder[len(prefix):].isdecimal()
    ]
    return Entity(value, _placeholder_for(kind, max(ordinals, default=0) + 1), kind, added)


def _git(subcommand: list[str], target: Path) -> int:
    """Run one local, offline git query about *target* and return its exit code."""
    try:
        return subprocess.run(
            ["git", *subcommand, "--", target.name],
            cwd=target.parent, capture_output=True, timeout=30, check=False
        ).returncode
    except (OSError, subprocess.TimeoutExpired) as error:
        raise EvattError(
            f"cannot ask git about {target.name} in {target.parent}: {error}; "
            "put git on PATH and re-run once the repository is idle"
        ) from error


def require_gitignored(map_path: Path, description: str = "the entity map") -> None:
    """Refuse to proceed unless git both ignores the path and does not track it.

    Git owns gitignore semantics: negation, precedence, .git/info/exclude, and
    the fact that an already-tracked file is not ignored at all. Asking git is
    the only answer that matches what a commit would do, and every uncertainty
    fails closed, because the map is the key. ``save`` calls this on the
    temporary it is about to create, which is why *map_path* need not exist yet.

    *description* names what is being refused, because this guard now covers
    three different files. The map and its temporaries are the key; the CLI's triage
    file is a worklist quoting whole residual lines about a real document, and
    telling an operator that "the entity map must never be committed" about a
    path ending ``.triage.md`` sends them to fix the wrong file.
    """
    target = map_path.resolve()
    tracked = _git(["ls-files", "--error-unmatch"], target)
    ignored = _git(["check-ignore", "--quiet"], target)
    if tracked == 0:
        problem = "git already tracks it; run git rm --cached, then purge it from history"
    elif tracked == 128:
        problem = "it is not inside a git work tree; move it into the repository that ignores it"
    elif tracked != 1 or ignored not in (0, 1):
        problem = f"git answered unexpectedly: ls-files {tracked}, check-ignore {ignored}"
    elif ignored == 1:
        problem = "git does not ignore it; add a covering rule to .gitignore"
    else:
        return
    raise EvattError(f"{description} must never be committed. {target}: {problem}")
