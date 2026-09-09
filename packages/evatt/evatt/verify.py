"""Re-scan an already-sanitised file and report every reason it is not clean.

This is the command to run in front of someone else. What it does is re-run
the same detection over the output: ``structured_spans``, the map sweep, the
carried-placeholder sweep and ``residual``, which are the same functions pass
one, pass two and the residual sweep use.

That catches a redaction application bug, a document redacted against a
different map, a half-redacted file and an output somebody edited by hand. It
does not catch a detection bug. Anything the detector cannot see on the way in
it cannot see on the way out either, so a clean verify says the output agrees
with the detector, not that the output is clean. This file used to claim the
detection was repeated "independently", and a mapped name in lower case was
the counter-example: it was replaced by nothing, reported by nothing and
called clean here.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from .entities import _PREFIX, Entity
from .patterns import PLACEHOLDER, structured_spans, value_pattern
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
    """True when *value* stands in *text* as a whole word, as pass two matches it.

    "As pass two matches it" is the whole point, so the pattern comes from
    ``patterns.value_pattern`` rather than being compiled again here. While
    each module built its own, this one was case-sensitive too, so a leaked
    lower-case client name was invisible to the command whose job is to say a
    file is not ready to send.
    """
    return value_pattern(value).search(text) is not None


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

    CRLF is normalised the same way ``redact`` normalises it, and for the same
    reason. Inheriting the fix through the ``redact`` call below is not enough:
    ``structured_spans`` runs here over the raw text, so a tax file number
    wrapped across a CRLF break was reported in an LF copy of a document and
    passed clean in the CRLF copy. Normalising also settles the cosmetic half,
    a name reported as "Jane\\r\\nRoe" against the LF copy's "Jane\\nRoe", which
    is what stops the two copies giving different findings for the same file.
    """
    text = text.replace("\r\n", "\n")
    found = [Finding(kind, value) for _start, _end, kind, value in structured_spans(text)]
    for entity in entities:
        if _mentions(text, entity.value):
            found.append(Finding(entity.kind, entity.value))
    found.extend(_carried_placeholders(text, entities))
    sanitised, _counts = redact(text, entities, strict=False)
    found.extend(Finding(unknown.kind, unknown.value) for unknown in residual(sanitised))
    return tuple(found)
