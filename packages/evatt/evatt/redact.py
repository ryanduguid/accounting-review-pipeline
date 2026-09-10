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

from .entities import _PREFIX, Entity
from .errors import Halt
from .patterns import (
    ADDRESS,
    DOB,
    PLACEHOLDER,
    PLACEHOLDER_CI,
    person_name_spans,
    structured_spans,
    value_pattern,
)

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

    ``patterns.value_pattern`` is what a value is matched with, rather than a
    pattern compiled here. This function once compiled ``re.escape(value)``
    case-sensitively, which meant a mapped name written in lower case was
    replaced by nothing while the residual sweep, which requires a capital,
    could not report it either. One definition, read by ``verify`` as well, is
    what closes that gap for good.
    """
    found: list[tuple[int, int, int, Entity]] = []
    for priority, entity in enumerate(entities):
        # ``load`` rejects every entry these guards skip, but ``redact`` takes
        # any Sequence[Entity] and a caller can build one by hand, so the map
        # gate is not the only place they have to be stopped. The value is
        # checked first and the placeholder second, and the two are meant to be
        # read as a pair: an entry is only used when both halves of it are ones
        # this package would have minted itself.
        #
        # An empty value reaches re.escape as "", giving "(?<!\\w)(?!\\w)",
        # which matches at every non-word boundary and scatters the placeholder
        # through the whole document.
        #
        # A value shaped like an assigned placeholder rewrites what pass one
        # has just written: Entity("TFN_01", "CLIENT_07", ...) destroys the only
        # record that a tax file number was there and leaves a manifest
        # counting a client that never appeared.
        #
        # The shape test is PLACEHOLDER_CI, matching the case-insensitive
        # pattern ``value_pattern`` compiles below. Case-sensitive, it let
        # "tfn_01" straight through and did exactly that damage, and ``load``
        # accepted the same value, so the map gate was no backstop either.
        if (
            not isinstance(entity.value, str)
            or not entity.value.strip()
            or PLACEHOLDER_CI.search(entity.value)
        ):
            continue
        # The mirror of the guard above, on the other half of the entry, and
        # skipping for the same reason: only a placeholder ``restore`` can
        # reverse is ever emitted.
        #
        # Entity("Jane Roe", "TFN_01", ...) is the case that matters. Nothing
        # here validated the placeholder, so the name was replaced by TFN_01,
        # ``restore._restorable`` then refused to reverse it because reversing a
        # structured placeholder is what would undo the one-way guarantee, and
        # ``verify`` read TFN_01 as ordinary one-way output and called the file
        # clean. The name was gone, unrestorable and invisible to the command
        # whose job is to say a file is not ready.
        #
        # ``PLACEHOLDER.fullmatch`` is the same shape test ``restore`` applies,
        # so the two agree on what a placeholder is, and it also covers an empty
        # or whitespace-only placeholder, which would otherwise be inserted at
        # every match and leave the value replaced by nothing.
        #
        # The prefix has to be the one ``assign`` mints for this entry's kind.
        # A kind and a prefix that disagree are what ``load`` rejects, because
        # the manifest counts by kind while the document carries the prefix, and
        # a later ``assign`` reading the map by prefix would mint the same
        # placeholder again for someone else.
        if (
            not isinstance(entity.placeholder, str)
            or not isinstance(entity.kind, str)
            or PLACEHOLDER.fullmatch(entity.placeholder) is None
            or entity.placeholder.rsplit("_", 1)[0] != _PREFIX.get(entity.kind)
        ):
            continue
        for match in value_pattern(entity.value).finditer(text):
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


def _input_placeholders(text: str, redacted: str) -> tuple[Unknown, ...]:
    """Report placeholder-shaped text that was in the input before any pass ran.

    Such text is either a document that has already been through this boundary
    or a real string that collides with the naming scheme, and both are unsafe
    to redact silently. Pass one would mint its own CLIENT_01 beside the input's,
    and ``restore`` would then write a real client name into a position where it
    never appeared. The residual sweep cannot catch this, because it runs on the
    output, where placeholders are exactly what is expected.

    Which tokens count is decided from *text*, because that is the question
    being asked. Where they are quoted from is *redacted*, because the context
    ends up in a triage file in the directory the operator sends from, and
    quoting the input put a real tax file number, an email address and a mapped
    client name in plaintext there, on the same line the sweep reported in
    redacted form two lines above. Every context this module produces is now
    post-redaction, the residual sweep's included.

    A carried token normally survives both passes untouched, so it can be found
    again in *redacted* and quoted exactly where it stands. It survives because
    pass two skips any entity whose value is placeholder-shaped and pass one
    matches digits, not words. The exception is a token whose own digits form a
    valid identifier, "MEDICARE_2123456701" being the reachable one: pass one
    replaces the digits and the token is gone from the output. Losing the halt
    there would be worse than quoting an approximate line, so such a token is
    still reported, against the redacted line standing at its input line number.
    """
    carried = {match.group(0) for match in PLACEHOLDER.finditer(text)}
    if not carried:
        return ()
    lines, starts = _lines_and_starts(redacted)
    found: dict[tuple[int, str], Unknown] = {}
    for match in PLACEHOLDER.finditer(redacted):
        token = match.group(0)
        if token not in carried:
            continue
        number, context = _locate(lines, starts, match.start())
        found.setdefault((number, token), Unknown("placeholder", token, number, context))
    survived = {token for _number, token in found}
    if len(survived) < len(carried):
        input_lines, input_starts = _lines_and_starts(text)
        for match in PLACEHOLDER.finditer(text):
            token = match.group(0)
            if token in survived:
                continue
            number, _raw = _locate(input_lines, input_starts, match.start())
            context = lines[number - 1].strip() if 0 < number <= len(lines) else ""
            found.setdefault((number, token), Unknown("placeholder", token, number, context))
    return tuple(found[key] for key in sorted(found))


def redact(
    text: str, entities: Sequence[Entity], *, strict: bool = True
) -> tuple[str, dict[str, int]]:
    """Return the sanitised text, always LF, and a manifest of counts by kind.

    The manifest carries counts only. Putting values in it would defeat the
    purpose of the file it accompanies.

    CRLF is normalised to LF here, before any pass runs, because every pass
    downstream works in LF only. The detection patterns separate digit groups
    with ``[\\s-]?``, which is exactly one character, so a CRLF pair inside a
    wrapped identifier matched nothing: "TFN: 123 456\\r\\n782" came back whole
    with an empty manifest, while the same document saved with LF endings was
    redacted and counted.

    The guard sits in this function rather than in a caller because ``redact``
    is the public entry every caller routes through, and a guard in one caller
    leaves every other one, a test, a hook, another tool, with the silent
    under-detection. Widening the separator instead would have been
    seven edits across the detection core, each free to drift from the others.

    Normalising costs the caller nothing it was promised: this function owns
    detection, not byte fidelity. Line numbers in a ``Halt`` are unchanged,
    because collapsing CRLF removes no break. Giving a document its original
    endings back is the CLI's job, and ``cli._read`` and ``cli._write`` own it.
    """
    text = text.replace("\r\n", "\n")
    redacted, structured_counts = _replace_structured(text)
    redacted, entity_counts = _replace_entities(redacted, entities)
    unknowns = _input_placeholders(text, redacted) + residual(redacted)
    if unknowns and strict:
        raise Halt(unknowns)
    manifest: Counter = Counter()
    manifest.update(structured_counts)
    manifest.update(entity_counts)
    return redacted, dict(manifest)
