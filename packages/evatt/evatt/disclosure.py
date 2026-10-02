"""Local disclosure evidence, without granting or authenticating permission.

Callers supply entries from ``entities.load`` and independently selected
destination and decision identifiers. Keep the encoded record local: its map
digest can be compared with candidate maps, and its identifiers are metadata.
An outer sender must authenticate the external decision and use the returned
bytes, rather than reopening the source or adding unchecked message content.
"""
from __future__ import annotations

import hashlib
import json
import re
from typing import Sequence

from . import entities as entities_module
from .entities import Entity
from .errors import EvattError
from .verify import findings
from .version import __version__

SCHEMA = "evatt.disclosure-evidence/v1"
MAX_RECORD_BYTES = 4096
_FIELDS = {"schema", "output_sha256", "entity_map_sha256", "tool_version",
           "destination", "decision_ref"}
_DESTINATION = re.compile(r"[a-z0-9][a-z0-9._:-]{0,127}")
_DECISION_REF = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}")
_DIGEST = re.compile(r"[0-9a-f]{64}")


class DisclosureRefused(EvattError):
    """Valid input that has findings or does not match the recorded evidence."""


def _context(destination: str, decision_ref: str) -> None:
    if not isinstance(destination, str) or not _DESTINATION.fullmatch(destination):
        raise EvattError("destination must be a canonical opaque identifier of 1 to 128 characters")
    if not isinstance(decision_ref, str) or not _DECISION_REF.fullmatch(decision_ref):
        raise EvattError("decision reference must be an opaque identifier of 1 to 128 characters")


def _evidence(
    payload: bytes, entries: Sequence[Entity], destination: str, decision_ref: str
) -> dict[str, str]:
    _context(destination, decision_ref)
    if not isinstance(payload, bytes):
        raise EvattError("disclosure payload must be immutable bytes")
    if payload.startswith(b"\xef\xbb\xbf"):
        raise EvattError("disclosure payload must be UTF-8 without a BOM")
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError:
        raise EvattError("disclosure payload must be UTF-8 without a BOM") from None
    snapshot = tuple(entries)
    try:
        entities_module._check_sequence(snapshot)
    except EvattError:
        raise EvattError("invalid entity map for disclosure evidence") from None
    if findings(text, snapshot):
        raise DisclosureRefused("the disclosure payload has verifier findings")
    # Hash every validated field and retain entry order. Map-file whitespace
    # and object-key order do not affect the loaded entries or this digest.
    mapping = {
        "schema_version": entities_module.SCHEMA_VERSION,
        "entries": [
            {"value": entry.value, "placeholder": entry.placeholder,
             "kind": entry.kind, "added": entry.added}
            for entry in snapshot
        ],
    }
    encoded_map = json.dumps(mapping, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "schema": SCHEMA,
        "output_sha256": hashlib.sha256(payload).hexdigest(),
        "entity_map_sha256": hashlib.sha256(b"evatt.entity-map/v1\0" + encoded_map).hexdigest(),
        "tool_version": __version__,
        "destination": destination,
        "decision_ref": decision_ref,
    }


def create_record(
    payload: bytes, entries: Sequence[Entity], *, destination: str, decision_ref: str
) -> bytes:
    """Rescan and encode evidence for exact UTF-8 bytes, including empty input.

    No file is written and no external decision is checked. The caller must
    keep the returned metadata private and must not treat it as permission.
    """
    record = _evidence(payload, entries, destination, decision_ref)
    return (json.dumps(record, sort_keys=True, indent=2) + "\n").encode("utf-8")


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate record field")
        result[key] = value
    return result


def _invalid_constant(value: str) -> object:
    raise ValueError("non-finite record value")


def _parse(record: bytes) -> dict[str, str]:
    if not isinstance(record, bytes) or len(record) > MAX_RECORD_BYTES:
        raise EvattError("disclosure record must be at most 4096 bytes of UTF-8 JSON")
    try:
        document = json.loads(record.decode("utf-8"), object_pairs_hook=_unique_object,
                              parse_constant=_invalid_constant)
    except (UnicodeDecodeError, ValueError, RecursionError):
        raise EvattError("invalid disclosure record JSON") from None
    if not isinstance(document, dict) or set(document) != _FIELDS:
        raise EvattError("invalid disclosure record fields")
    if any(not isinstance(value, str) or len(value) > 128 for value in document.values()):
        raise EvattError("invalid disclosure record field types or lengths")
    if document["schema"] != SCHEMA:
        raise EvattError("unsupported disclosure record schema")
    if any(not _DIGEST.fullmatch(document[key]) for key in ("output_sha256", "entity_map_sha256")):
        raise EvattError("invalid disclosure record digest")
    _context(document["destination"], document["decision_ref"])
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.!+_-]{0,127}", document["tool_version"]):
        raise EvattError("invalid disclosure record tool version")
    return document


def check_record(
    payload: bytes, entries: Sequence[Entity], record: bytes, *,
    expected_destination: str, expected_decision_ref: str
) -> bytes:
    """Rescan and return the same bytes only when all supplied context matches.

    Expected identifiers must come from the caller's intended action, not from
    this unsigned record. Success authenticates neither the external decision
    nor the record. A future sender must independently enforce permission.
    """
    document = _parse(record)
    expected = _evidence(payload, entries, expected_destination, expected_decision_ref)
    if document != expected:
        raise DisclosureRefused("disclosure evidence does not match the supplied context")
    return payload
