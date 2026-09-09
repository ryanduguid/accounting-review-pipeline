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

import bisect
import re
from collections import Counter
from dataclasses import dataclass
from typing import Sequence

from .entities import Entity
from .errors import Halt
from .patterns import ADDRESS, DOB, PLACEHOLDER, person_name_spans, structured_spans

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


def _lines_and_starts(text: str) -> tuple[list[str], list[int]]:
    """Split *text* into lines with each line's offset into the whole text.

    ``splitlines`` does the splitting, so the reported line numbers stay exactly
    what they were when the sweep ran line by line, including its treatment of
    U+2028, VT, FF and NEL as breaks. The offsets come from ``keepends=True``,
    which is what lets a match found over the whole text be mapped back to a
    line without rescanning anything.
    """
    lines = text.splitlines(keepends=True)
    starts: list[int] = []
    offset = 0
    for line in lines:
        starts.append(offset)
        offset += len(line)
    return lines, starts


def _locate(lines: list[str], starts: list[int], position: int) -> tuple[int, str]:
    """The one-based line number holding *position*, and that line stripped."""
    if not starts:
        return 1, ""
    index = bisect.bisect_right(starts, position) - 1
    return index + 1, lines[index].strip()


def _blank(match: re.Match[str]) -> str:
    """Filler of the same length as the match, so masking cannot shift offsets."""
    return "#" * len(match.group(0))


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
    """Resolve every entity value over the whole text at once, then rebuild it once.

    Sorting by length and substituting one value at a time handles containment
    but not overlap. With "Jane Roe" and "Roe Holdings" both mapped, the longer
    value ran first over "Jane Roe Holdings" and left "Jane" behind: a one-token
    remnant of a real person that NAME cannot see and the residual sweep
    therefore never reports. Resolving all values to non-overlapping spans in
    one pass is the same shape as ``structured_spans``, and for the same reason:
    replacing in place and rescanning is how a redactor eats its own output.

    Leftmost-longest, with the map's own order as the final tie-break, so the
    result does not depend on how the caller happened to sort the map.
    """
    found: list[tuple[int, int, int, Entity]] = []
    for priority, entity in enumerate(entities):
        # ``load`` rejects an empty value, but ``redact`` takes any
        # Sequence[Entity] and a caller can build one by hand. An empty value
        # reaches re.escape as "", giving "(?<!\\w)(?!\\w)", which matches at
        # every non-word boundary and scatters the placeholder through the
        # whole document.
        if not entity.value.strip():
            continue
        pattern = re.compile(r"(?<!\w)" + re.escape(entity.value) + r"(?!\w)")
        for match in pattern.finditer(text):
            found.append((match.start(), match.end(), priority, entity))
    found.sort(key=lambda f: (f[0], -(f[1] - f[0]), f[2]))
    counts: Counter = Counter()
    pieces: list[str] = []
    cursor = 0
    for start, end, _priority, entity in found:
        if start < cursor:
            continue
        counts[entity.kind] += 1
        pieces.append(text[cursor:start])
        pieces.append(entity.placeholder)
        cursor = end
    pieces.append(text[cursor:])
    return "".join(pieces), counts


def residual(text: str) -> tuple[Unknown, ...]:
    """Report every candidate neither earlier pass recognised.

    This runs on the output of passes one and two, so anything it finds is by
    definition unclassified. It reports rather than guesses, and ``redact`` in
    strict mode turns any report into a halt. A detector that silently passes
    what it does not understand is the failure mode that leaks; one that stops
    and asks is safe even when its detection is crude.

    Addresses and dates are swept first and then masked out before the name
    sweep runs. Without the mask "12 Hunter Street" reports twice, once as an
    address and again as the person "Hunter Street", and a triage file that
    names a street as a human being teaches an operator to skim it. The mask
    character is not a word character, so masking cannot join two separated
    capitals into a name that was never in the text, and the mask is the same
    length as what it covers so that offsets still point where they did.

    Every pattern runs over the whole text, not line by line, and each match is
    mapped back to the line it starts on. NAME's ``\\s+`` matches a newline, so a
    per-line scan cannot see a name a hard wrap has split: "John\\nSmith" was
    returned unchanged with an empty manifest and no halt, and hard-wrapped
    markdown is the ordinary case for the documents this boundary exists for.
    """
    lines, starts = _lines_and_starts(text)
    found: list[Unknown] = []
    for kind, pattern in (("address", ADDRESS), ("date", DOB)):
        for match in pattern.finditer(text):
            number, context = _locate(lines, starts, match.start())
            found.append(Unknown(kind, match.group(0), number, context))
    masked = DOB.sub(_blank, ADDRESS.sub(_blank, text))
    # One report per name per line, which is what the line-by-line set gave and
    # what keeps a name repeated down a page from filling the triage file.
    names: dict[tuple[int, str], Unknown] = {}
    for start, _end, value in person_name_spans(masked):
        # NAME's trailing \b already stops a placeholder forming a name, so
        # this guard fires on nothing today. It stays because the property it
        # protects is that the sweep never halts on the redactor's own output,
        # and that must not rest on an incidental \b in a pattern this module
        # does not own.
        if PLACEHOLDER.search(value):
            continue
        number, context = _locate(lines, starts, start)
        names.setdefault((number, value), Unknown("name", value, number, context))
    found.extend(names[key] for key in sorted(names))
    return tuple(found)


def _input_placeholders(text: str) -> tuple[Unknown, ...]:
    """Report placeholder-shaped text that was in the input before any pass ran.

    Such text is either a document that has already been through this boundary
    or a real string that collides with the naming scheme, and both are unsafe
    to redact silently. Pass one would mint its own CLIENT_01 beside the input's,
    and ``restore`` would then write a real client name into a position where it
    never appeared. The residual sweep cannot catch this, because it runs on the
    output, where placeholders are exactly what is expected.
    """
    lines, starts = _lines_and_starts(text)
    found: list[Unknown] = []
    for match in PLACEHOLDER.finditer(text):
        number, context = _locate(lines, starts, match.start())
        found.append(Unknown("placeholder", match.group(0), number, context))
    return tuple(found)


def redact(
    text: str, entities: Sequence[Entity], *, strict: bool = True
) -> tuple[str, dict[str, int]]:
    """Return the sanitised text and a manifest of counts by kind.

    The manifest carries counts only. Putting values in it would defeat the
    purpose of the file it accompanies.
    """
    carried = _input_placeholders(text)
    text, structured_counts = _replace_structured(text)
    text, entity_counts = _replace_entities(text, entities)
    unknowns = carried + residual(text)
    if unknowns and strict:
        raise Halt(unknowns)
    manifest: Counter = Counter()
    manifest.update(structured_counts)
    manifest.update(entity_counts)
    return text, dict(manifest)
