"""The redaction passes.

Pass one replaces structured identifiers, confirmed by check digit, one-way.
Pass two replaces known entities from the map. Pass three sweeps for anything
left that looks like a person, an address or a date of birth, and halts rather
than guessing.

Structured identifiers are one-way on purpose. Nothing records what TFN_01
was. A model's answer never needs to echo a real tax file number back, so
keeping the values to reverse them would create a second copy of the most
sensitive material for no benefit.
"""
from __future__ import annotations

import re
from collections import Counter
from dataclasses import dataclass
from typing import Sequence

from .entities import Entity
from .errors import Halt
from .patterns import ADDRESS, DOB, PLACEHOLDER, person_names, structured_spans

_KIND_PREFIX = {
    "tfn": "TFN", "abn": "ABN", "acn": "ACN", "bsb": "BSB",
    "medicare": "MEDICARE", "email": "EMAIL", "phone": "PHONE",
}


@dataclass(frozen=True)
class Unknown:
    kind: str
    value: str
    line: int
    context: str


def _normalise(kind: str, value: str) -> str:
    """Collapse spelling differences so one identifier gets one placeholder."""
    if kind == "email":
        return value.strip().casefold()
    return "".join(c for c in value if c.isdigit())


def _replace_structured(text: str) -> tuple[str, Counter]:
    """Rebuild *text* once from the resolved spans, assigning ordinals in document order."""
    counts: Counter = Counter()
    assigned: dict[tuple[str, str], str] = {}
    pieces: list[str] = []
    cursor = 0
    for start, end, kind, value in structured_spans(text):
        key = (kind, _normalise(kind, value))
        placeholder = assigned.get(key)
        if placeholder is None:
            ordinal = len([k for k in assigned if k[0] == kind]) + 1
            placeholder = f"{_KIND_PREFIX[kind]}_{ordinal:02d}"
            assigned[key] = placeholder
        counts[kind] += 1
        pieces.append(text[cursor:start])
        pieces.append(placeholder)
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces), counts


def _replace_entities(text: str, entities: Sequence[Entity]) -> tuple[str, Counter]:
    """Replace longest values first so a contained name cannot win over its container."""
    counts: Counter = Counter()
    for entity in sorted(entities, key=lambda e: len(e.value), reverse=True):
        pattern = re.compile(r"(?<!\w)" + re.escape(entity.value) + r"(?!\w)")
        text, replaced = pattern.subn(entity.placeholder, text)
        if replaced:
            counts[entity.kind] += replaced
    return text, counts


def residual(text: str) -> tuple[Unknown, ...]:
    """Report every candidate neither earlier pass recognised.

    This runs on the output of passes one and two, so anything it finds is by
    definition unclassified. It reports rather than guesses, and ``redact`` in
    strict mode turns any report into a halt. A detector that silently passes
    what it does not understand is the failure mode that leaks; one that stops
    and asks is safe even when its detection is crude.

    Addresses and dates are swept first and then masked out of the line before
    the name sweep runs. Without the mask "12 Hunter Street" reports twice, once
    as an address and again as the person "Hunter Street", and a triage file
    that names a street as a human being teaches an operator to skim it. The
    mask character is not a word character, so masking cannot join two separated
    capitals into a name that was never in the text.
    """
    found: list[Unknown] = []
    for number, line in enumerate(text.splitlines(), start=1):
        context = line.strip()
        for kind, pattern in (("address", ADDRESS), ("date", DOB)):
            for match in pattern.finditer(line):
                found.append(Unknown(kind, match.group(0), number, context))
        masked = DOB.sub("#", ADDRESS.sub("#", line))
        for value in sorted(person_names(masked)):
            # NAME's trailing \b already stops a placeholder forming a name, so
            # this guard fires on nothing today. It stays because the property it
            # protects is that the sweep never halts on the redactor's own output,
            # and that must not rest on an incidental \b in a pattern this module
            # does not own.
            if PLACEHOLDER.search(value):
                continue
            found.append(Unknown("name", value, number, context))
    return tuple(found)


def redact(
    text: str, entities: Sequence[Entity], *, strict: bool = True
) -> tuple[str, dict[str, int]]:
    """Return the sanitised text and a manifest of counts by kind.

    The manifest carries counts only. Putting values in it would defeat the
    purpose of the file it accompanies.
    """
    text, structured_counts = _replace_structured(text)
    text, entity_counts = _replace_entities(text, entities)
    unknowns = residual(text)
    if unknowns and strict:
        raise Halt(unknowns)
    manifest: Counter = Counter()
    manifest.update(structured_counts)
    manifest.update(entity_counts)
    return text, dict(manifest)
