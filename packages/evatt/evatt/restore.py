"""Reverses the entity map over a model's answer, locally.

Named entities only. Structured identifiers were replaced one-way and no
record of their values exists, so a TFN, ABN, ACN, BSB or Medicare number
cannot come back through here however the answer is worded.
"""
from __future__ import annotations

import re
from typing import Callable, Sequence

from .entities import _PREFIX, Entity
from .patterns import PLACEHOLDER, PLACEHOLDER_BOUNDARY

# The four prefixes ``assign`` mints, read from the table it mints them with so
# the two cannot drift, exactly as ``verify`` reads them. The structured
# prefixes are deliberately absent: TFN_01 is what a clean redacted file looks
# like, and nothing reverses it.
_ENTITY_PREFIXES = frozenset(_PREFIX.values())


def _literally(value: str) -> Callable[[re.Match[str]], str]:
    """A replacement function that writes *value* exactly as it stands.

    ``re.sub`` reads backslashes and ``\\g`` in a string replacement as
    references. A real value is arbitrary text off the map, so "Smith \\ Co"
    would raise, and a value holding "\\1" would expand to a group that is not
    there. A function replacement is never scanned for escapes.
    """
    return lambda _match: value


def _restorable(entity: Entity) -> bool:
    """True when *entity* is one ``restore`` may reverse.

    ``load`` rejects every entity this refuses, but ``restore`` takes any
    ``Sequence[Entity]`` and a caller can build one by hand, so the map gate is
    not the only place they have to be stopped. ``redact._replace_entities``
    guards its own input for the same reason.

    The shape test covers an empty or whitespace-only placeholder as well.
    Such a placeholder reaches ``re.escape`` as nothing, the boundaries around
    nothing match at every token edge, and the real value is then scattered
    through the whole answer.

    The prefix test is what keeps the one-way guarantee. An entity carrying the
    value "123 456 782" on the placeholder TFN_01 is the case that matters:
    pass one replaced that tax file number and kept no record of it, and
    honouring a hand-built entry pointing back at TFN_01 would put the number
    into the answer. The claimed ``kind`` is not consulted, because a
    hand-built entity can claim anything; the placeholder is the thing being
    matched, so the placeholder is what is checked.

    A value carrying another entity's placeholder is refused too. The loop is
    sequential, so a value holding PERSON_01 is itself rewritten by the entry
    that owns PERSON_01, and two real values end up spliced together at a
    position neither of them occupied.
    """
    return (
        PLACEHOLDER.fullmatch(entity.placeholder) is not None
        and entity.placeholder.rsplit("_", 1)[0] in _ENTITY_PREFIXES
        and not PLACEHOLDER.search(entity.value)
    )


def restore(text: str, entities: Sequence[Entity]) -> str:
    """Substitute each placeholder back to its real value.

    The boundaries are symmetric, ``(?<![A-Za-z0-9_])`` before and
    ``(?![A-Za-z0-9_])`` after, and both sides are load-bearing.

    Without the right one, replacing CLIENT_10 also rewrites the first nine
    characters of CLIENT_100, and the map holds both the moment a client list
    passes ninety-nine entries. Without the left one, "PRIOR_CLIENT_01" and
    "XCLIENT_01", which are ordinary shapes for a ledger column, a code fence
    or a heading, become a real client name in a position it never occupied.
    That direction is the dangerous one, because neither guard reports it:
    ``redact._input_placeholders`` and ``verify._carried_placeholders`` both
    read ``PLACEHOLDER``, which carries the left boundary this function once
    dropped. Taking both sides from ``PLACEHOLDER_BOUNDARY`` is what stops the
    three drifting apart again.

    Pinning the trailing side on any word character rather than only a digit
    buys one thing more: "CLIENT_01s" is left standing instead of restored to
    "Sample Holdings Pty Ltds", so a token that is not a placeholder stays
    visibly unrestored rather than silently wrong.
    """
    for entity in entities:
        if not _restorable(entity):
            continue
        pattern = re.compile(
            f"(?<!{PLACEHOLDER_BOUNDARY}){re.escape(entity.placeholder)}"
            f"(?!{PLACEHOLDER_BOUNDARY})"
        )
        text = pattern.sub(_literally(entity.value), text)
    return text
