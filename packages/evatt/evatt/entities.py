"""The local entity map: real value to stable placeholder.

The map is the key. It stays on disk, gitignored, and is never transmitted.
Placeholders are typed and stable so that the receiving model can still reason
across a document, and so ``restore`` can put real names back into its answer.

Ordinals are stored rather than derived at read time. Deriving them from
iteration order would make output depend on dictionary ordering, and the
determinism property in the test suite exists to catch exactly that.
"""
from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, Sequence

from .errors import EvattError

SCHEMA_VERSION = 1
KINDS = ("client", "person", "staff", "entity")
_PREFIX = {"client": "CLIENT", "person": "PERSON", "staff": "STAFF", "entity": "ENTITY"}
_REQUIRED = {"value", "placeholder", "kind", "added"}
_ADDED = re.compile(r"\d{4}-\d{2}-\d{2}\Z")


@dataclass(frozen=True)
class Entity:
    value: str
    placeholder: str
    kind: str
    added: str


def _placeholder_for(kind: str, ordinal: int) -> str:
    return f"{_PREFIX[kind]}_{ordinal:02d}"


def load(path: Path) -> tuple[Entity, ...]:
    """Load and strictly validate the map. Every rejection is a hard error."""
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvattError(f"cannot read entity map {path}: {error}") from error
    if not isinstance(document, dict) or set(document) != {"schema_version", "entries"}:
        raise EvattError("entity map must hold exactly schema_version and entries")
    if document["schema_version"] != SCHEMA_VERSION:
        raise EvattError(f"unsupported entity map schema {document['schema_version']!r}")
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
        if not isinstance(value, str) or not value.strip():
            raise EvattError("entity value must be a non-empty string")
        if kind not in KINDS:
            raise EvattError(f"unknown entity kind {kind!r}")
        if not isinstance(added, str) or not _ADDED.fullmatch(added):
            raise EvattError(f"entity added date must be YYYY-MM-DD, got {added!r}")
        if not isinstance(placeholder, str) or not placeholder.startswith(_PREFIX[kind] + "_"):
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
    """Write the map with a trailing newline and stable key order."""
    document = {
        "schema_version": SCHEMA_VERSION,
        "entries": [
            {"value": e.value, "placeholder": e.placeholder, "kind": e.kind, "added": e.added}
            for e in entities
        ],
    }
    path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def assign(entities: Sequence[Entity], value: str, kind: str, added: str) -> Entity:
    """Return a new Entity with the next free ordinal for *kind*.

    Pure: it does not mutate *entities*. The caller appends and saves.
    """
    if kind not in KINDS:
        raise EvattError(f"unknown entity kind {kind!r}")
    used = {e.placeholder for e in entities}
    ordinal = 1
    while _placeholder_for(kind, ordinal) in used:
        ordinal += 1
    return Entity(value=value, placeholder=_placeholder_for(kind, ordinal), kind=kind, added=added)


def _ancestor_gitignores(map_path: Path) -> Iterator[Path]:
    """Yield the .gitignore files beside and above the map, nearest first."""
    for directory in [map_path.parent, *map_path.parent.parents]:
        candidate = directory / ".gitignore"
        if candidate.exists():
            yield candidate


def _patterns(gitignore: Path) -> set[str]:
    """Read the active patterns from a .gitignore, one per line.

    Splitting the file on all whitespace would turn a commented-out rule such
    as ``# entities.json`` into the two tokens ``#`` and ``entities.json``, so
    a disabled rule would satisfy the guard. Parse line by line instead.
    """
    active = set()
    for line in gitignore.read_text(encoding="utf-8").splitlines():
        pattern = line.strip()
        if pattern and not pattern.startswith("#"):
            active.add(pattern)
    return active


def require_gitignored(map_path: Path) -> None:
    """Refuse to proceed unless a .gitignore beside or above the map covers it.

    This is a cheap structural check, not a call into git. The failure it
    prevents is the map being committed, and a plain-text scan of the
    .gitignore files on the path to the root catches that.
    """
    name = map_path.name
    for candidate in _ancestor_gitignores(map_path):
        patterns = _patterns(candidate)
        wildcard = "*.entities.json" in patterns and name.endswith(".entities.json")
        if name in patterns or wildcard:
            return
    raise EvattError(
        f"{map_path} is not covered by any .gitignore on the path to the filesystem root; "
        "the entity map is the key and must never be committed"
    )
