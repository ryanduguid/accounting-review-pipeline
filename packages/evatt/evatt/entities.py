"""The local entity map: real value to stable placeholder.

The map is the key. It stays on disk, gitignored, and is never transmitted.
Placeholders are typed and stable so that the receiving model can still reason
across a document, and so ``restore`` can put real names back into its answer.

Ordinals are stored rather than derived at read time. Deriving them from
iteration order would make output depend on dictionary ordering, and the
determinism property in the test suite exists to catch exactly that. They are
also monotonic: an ordinal freed by a deletion is never handed out again,
because a placeholder already written into a redacted document must not come
to mean somebody else.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Sequence

from .errors import EvattError

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


def _check_fields(value: object, kind: object, added: object) -> None:
    """Validate the fields ``load`` and ``assign`` share, so the two cannot drift.

    An ``assign`` that mints an entity ``load`` would later reject turns the map
    into a file that cannot be read back, and the map is the only copy of the key.
    """
    if not isinstance(value, str) or not value.strip():
        raise EvattError("entity value must be a non-empty string")
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
    seen_values: set[str] = set()
    seen_placeholders: set[str] = set()
    for entry in document["entries"]:
        if not isinstance(entry, dict) or set(entry) != _REQUIRED:
            raise EvattError(f"entity map entry must hold exactly {sorted(_REQUIRED)}")
        value, placeholder = entry["value"], entry["placeholder"]
        kind, added = entry["kind"], entry["added"]
        _check_fields(value, kind, added)
        # The full shape, not just the prefix: a map holding both CLIENT_1 and
        # CLIENT_10 would let naive replacement corrupt the longer placeholder.
        if not isinstance(placeholder, str) or not re.fullmatch(
            rf"{_PREFIX[kind]}_[0-9]{{2,}}", placeholder
        ):
            raise EvattError(f"placeholder {placeholder!r} does not match kind {kind!r}")
        if value in seen_values:
            raise EvattError(f"duplicate entity value {value!r}")
        if placeholder in seen_placeholders:
            raise EvattError(f"duplicate placeholder {placeholder!r}")
        seen_values.add(value)
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
    Ordinals are max+1 within the kind, never the lowest free number, so a
    deletion cannot hand a retired placeholder to somebody new.

    Pure: it does not mutate *entities*. The caller appends and saves.
    """
    _check_fields(value, kind, added)
    for existing in entities:
        if existing.value == value:
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


def require_gitignored(map_path: Path) -> None:
    """Refuse to proceed unless git both ignores the map and does not track it.

    Git owns gitignore semantics: negation, precedence, .git/info/exclude, and
    the fact that an already-tracked file is not ignored at all. Asking git is
    the only answer that matches what a commit would do, and every uncertainty
    fails closed, because the map is the key. ``save`` calls this on the .tmp
    path too, which is why *map_path* need not exist yet.
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
    raise EvattError(f"the entity map must never be committed. {target}: {problem}")
