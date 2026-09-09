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
import subprocess
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from types import MappingProxyType
from typing import Mapping, Sequence

from .errors import EvattError
from .patterns import PLACEHOLDER_CI

SCHEMA_VERSION = 1
KINDS = ("client", "person", "staff", "entity")
_PREFIX = {"client": "CLIENT", "person": "PERSON", "staff": "STAFF", "entity": "ENTITY"}
_REQUIRED = {"value", "placeholder", "kind", "added"}


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
) -> None:
    """Validate the fields ``load`` and ``assign`` share, so the two cannot drift.

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
    """
    if not isinstance(value, str) or not value.strip():
        raise EvattError("entity value must be a non-empty string")
    if PLACEHOLDER_CI.search(value):
        raise EvattError(f"entity value {value!r} is shaped like an assigned placeholder")
    if kind not in KINDS:
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
        _check_fields(value, kind, added, seen_values)
        # The full shape, not just the prefix: a map holding both CLIENT_1 and
        # CLIENT_10 would let naive replacement corrupt the longer placeholder.
        if not isinstance(placeholder, str) or not re.fullmatch(
            rf"{_PREFIX[kind]}_[0-9]{{2,}}", placeholder
        ):
            raise EvattError(f"placeholder {placeholder!r} does not match kind {kind!r}")
        if placeholder in seen_placeholders:
            raise EvattError(f"duplicate placeholder {placeholder!r}")
        seen_values[_fold(value)] = value
        seen_placeholders.add(placeholder)
        loaded.append(Entity(value=value, placeholder=placeholder, kind=kind, added=added))
    return tuple(loaded)


def save(path: Path, entities: Sequence[Entity]) -> None:
    """Write the map atomically, with a trailing newline and stable key order.

    A plain write truncates before it fills. If that is interrupted, the only
    copy of the key is gone, so write a neighbouring temporary file and rename.

    The temporary holds the same real values as the map, so it is held to the
    same standard before a byte is written to it. The ``finally`` clause removes
    it after any Python-level failure, but a hard kill, a container stop or a
    power loss all leave it on disk, so "it is short lived" is not a reason to
    let a plaintext copy of the key sit at a committable path. Guarding the
    temporary rather than naming ``.tmp`` in one .gitignore is what makes this
    correct wherever the map lives, and it means ``save`` requires a work tree.

    The temporary is flushed and fsynced before the rename, because a rename
    that reaches the disk ahead of the data it points at produces exactly the
    truncated map the temporary exists to prevent.
    """
    require_gitignored(path)
    temporary = path.with_name(path.name + ".tmp")
    require_gitignored(temporary)
    document = {
        "schema_version": SCHEMA_VERSION,
        "entries": [
            {"value": e.value, "placeholder": e.placeholder, "kind": e.kind, "added": e.added}
            for e in entities
        ],
    }
    text = json.dumps(document, indent=2, ensure_ascii=False) + "\n"
    try:
        with temporary.open("w", encoding="utf-8") as handle:
            handle.write(text)
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
    fails closed, because the map is the key. ``save`` calls this on the .tmp
    path too, which is why *map_path* need not exist yet.

    *description* names what is being refused, because this guard now covers
    three different files. The map and its .tmp are the key; the CLI's triage
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
