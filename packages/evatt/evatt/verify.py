"""Re-scan an already-sanitised file and report every reason it is not clean.

This is the command to run in front of someone else. It repeats the whole
detection independently of the redaction path, so a bug that made redaction
skip something has a second chance to be caught before the file is sent.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Sequence

from .entities import _PREFIX, Entity
from .patterns import PLACEHOLDER, structured_spans
from .redact import redact, residual

# The four prefixes ``assign`` mints, taken from the table it mints them with so
# the two cannot drift. The structured prefixes are deliberately not here: a
# clean redacted file is full of TFN_01 and EMAIL_02, and nothing reverses them.
_ENTITY_PREFIXES = frozenset(_PREFIX.values())


@dataclass(frozen=True)
class Finding:
    kind: str
    value: str


def _mentions(text: str, value: str) -> bool:
    """True when *value* stands in *text* as a whole word, as pass two matches it."""
    return re.search(r"(?<!\w)" + re.escape(value) + r"(?!\w)", text) is not None


def _carried_placeholders(text: str, entities: Sequence[Entity]) -> list[Finding]:
    """Report an entity placeholder the input wrote itself rather than redaction.

    ``redact`` detects this at the input, but only strict mode acts on it, and
    ``findings`` needs the sanitised form, which it can only get with
    ``strict=False``. So the sweep is repeated here over the raw input, on the
    two shapes that survive being told nothing about which pass wrote what:

    * a placeholder the map does not assign, which ``restore`` cannot reverse.
      Either the file was redacted against a different map or the string is a
      literal that collides with the naming scheme, and both mean the file and
      the key in hand disagree about what the file says.
    * a placeholder standing beside the very value it stands for, which is a
      half-redacted file. Redaction had no reason to mint it there, because the
      real value is still sitting in the text.

    What no sweep can see is a lone literal CLIENT_01 in a document holding
    nothing else: at that point it is character for character a correctly
    redacted document, and ``restore`` will write a real client name into it.
    That case is why ``redact`` halts on an input placeholder in strict mode
    rather than leaving it for this command to find later. The mitigation is
    conditional on that halt being reachable: it holds only while every caller
    on the operator path redacts strictly, so the CLI must never expose a
    non-strict redact.
    """
    assigned = {entity.placeholder: entity for entity in entities}
    found: list[Finding] = []
    # PLACEHOLDER has no capturing group, so findall returns whole matches, and
    # dict.fromkeys keeps one report per distinct placeholder in document order.
    for token in dict.fromkeys(PLACEHOLDER.findall(text)):
        if token.rsplit("_", 1)[0] not in _ENTITY_PREFIXES:
            continue
        entity = assigned.get(token)
        if entity is None or _mentions(text, entity.value):
            found.append(Finding("placeholder", token))
    return found


def findings(text: str, entities: Sequence[Entity]) -> tuple[Finding, ...]:
    """Return every surviving identifier, mapped entity and unclassified candidate.

    The residual sweep runs over the redacted form, not the raw text. Running
    it raw would report a known client twice, once as a mapped entity and
    again as an unclassified name, which buries the genuinely unknown items
    the operator has to act on.
    """
    found = [Finding(kind, value) for _start, _end, kind, value in structured_spans(text)]
    for entity in entities:
        if _mentions(text, entity.value):
            found.append(Finding(entity.kind, entity.value))
    found.extend(_carried_placeholders(text, entities))
    sanitised, _counts = redact(text, entities, strict=False)
    found.extend(Finding(unknown.kind, unknown.value) for unknown in residual(sanitised))
    return tuple(found)
